# Experiment log

Newest last. Format and rules: `README.md`.

### E000 baseline
- Branch / parent: feature/rust-kernel at c6cf926 (+ bench score line) / none
- Hypothesis: none; this is the reference point
- Change: none
- Result: score **0.8896**
  - Per corpus `vs_dict`: md 1.408, pdf 1.016, office 1.321, invoices 0.700, synthetic-history 0.308, repo-history 1.218
  - Bytes after repack: md 108,244; pdf 25,043; office 516,710; invoices 9,470; synthetic 18,680; repo 1,572,127
  - Total time 20.7 s
- Verdict: reference
- Open gaps, largest first:
  - md: 1.41. Many tiny files; tree metadata is part of it.
  - office: 1.32. The trained dictionary for the members is only 6.9 KB.
  - repo-history: 1.22.
  - Literature and hypotheses for these gaps: `docs/research/office-gap.md`, `docs/research/kernel-direction.md`.

### E001 accounting
- Branch / parent: exp/001-accounting / E000
- Hypothesis: much of the md/office/repo-history gap is bookkeeping the baseline never pays (trees, derivation records), not weaker content compression (source: `docs/research/office-gap.md` H6)
- Change: `examples/bench.rs` reports `by_kind`: count and stored bytes per encoding tag (B/Z/Y content, T tree, C commit, D derivation, L claim). No kernel change.
- Result: score 0.8896 -> 0.8896 (measurement only). Stored bytes by kind (content = B+Y+Z):
  | corpus | content | trees | derivations | baseline |
  |---|---:|---:|---:|---:|
  | md | 79,940 | 28,263 (1) | 0 | 76,892 |
  | office | 305,183 | 20,601 (1) | 190,885 (489) | 391,236 |
  | repo-history | 1,114,603 | 438,776 (7) | 18,123 (75) | 1,290,663 |
  - Content alone already beats the baseline on office (0.78) and repo-history (0.86), and is within 4% on md.
  - The gaps are structure: md trees 26% of stored bytes; office derivation records 37% (≈390 B each, mostly 33 B per member reference); repo-history trees 28%. Trees and derivations are stored uncompressed.
  - Unscored probe (zstd -19 per object, not committed): md trees 28,263 -> 14,021; office derivations 190,885 -> 168,070; repo-history trees 438,776 -> 198,345. Per-object compression helps trees; derivations barely shrink because their repeats (the same member ids) are across objects, not within one.
- Verdict: kept (adds a capability, score holds). H6 confirmed. Next: compress structural objects, with a dictionary so ids repeated across objects compress (E002).

### E002 compressed-objects
- Branch / parent: exp/002-compressed-objects / E001
- Hypothesis: storing trees, commits, derivations and claims compressed, against a dictionary that is also trained on them, removes most of the structural overhead E001 measured (source: E001 measurement and probe; `docs/research/office-gap.md` sources 6 and 9 on shared context)
- Change:
  - `src/dict.rs`: objects get storage encodings `lower(tag) 0 zstd(rest)` and `lower(tag) 1 dict[32] zstd_dict(rest)`. Ids stay `BLAKE3(canonical)`; `read_object` decodes. `encode_plain` shares one `compress` helper and is unchanged in behaviour.
  - `src/repo.rs` `write` stores objects in their smallest encoding. Ledger, `get`, `links`/`used_by`, claims and gc read through `read_object` or `object_tag`; gc keeps dictionaries that any live object needs.
  - `src/repack.rs` adds trees, commits, claims and fact derivations to the dictionary samples, then re-encodes those objects against the new dictionary.
  - Tests: new law `compressed_objects_keep_identity_and_detect_corruption`. The no-level-limit template test asserted that repack *chooses* a multi-level plan. It is split into a unit test that the induced plan still layers templates (passes) and an integration test that revisions repack into templates.
- Result: score 0.8896 -> **0.8157**; per corpus vs_dict: md 1.408->1.222, pdf 1.016->0.962, office 1.321->1.224, invoices 0.700->0.678, synthetic 0.308->0.289, repo 1.218->1.047; time 20.8 s -> 20.6 s
  - Bytes: md 108,244->93,921; pdf 25,043->23,710; office 516,710->479,009; invoices 9,470->9,164; synthetic 18,680->17,525; repo 1,572,127->1,350,803
  - Trees roughly halve (md 28,263->14,258, repo-history 438,776->202,230); they remain mostly 32 B ids.
  - Office derivations: 190,885 -> 190,192 across 614 records (353 stay canonical; their member ids are unique to each record), so the gain on office is mostly from content stored at ingest.
  - Office repack is not applied: the best re-plan (dictionary, no templates: 472,474) is larger than the ingest state (468,166). The trained dictionary is 6.6 KB, the undersized dictionary of office-gap H2.
  - Synthetic-history now keeps the non-induced plan (15,016 vs 16,171 for the induced one), because compressed records changed relative costs. The induced plan still nests templates (unit test).
- Verdict: kept. Score improves on every corpus; gates pass.
- Next gaps: md 1.22 (tree ids and per-blob framing: office-gap H1), office 1.22 (dictionary sizing: H2; 32 B per member reference in zip records), repo-history 1.05 (trees: 7 full listings, one per snapshot, unshared).

### E003 dictionary
- Branch / parent: exp/003-dictionary / E002
- Hypothesis: the shared dictionary is undersized and mis-shaped. Measuring trained (COVER) and raw-content candidates at several sizes beats one COVER call at total/10 (source: `docs/research/office-gap.md` H2; zdict.h's ~100x guidance; Hoobin et al. PVLDB 2011 raw dictionaries)
- Probe (unscored, estimate = dictionary + per-sample best encoding):
  - On office, COVER collapses: asked for 74 KB or 110 KB, it returns 5.7–6.6 KB, estimated at 540–549 KB (no dictionary: 549 KB). A raw dictionary of 1 KiB pieces at even steps: 336 KB at 72 KB.
  - On md and repo-history, COVER is slightly better than raw.
- Variants (all exact and fsck-clean):
  | variant | change | score | md | office | invoices | time |
  |---|---|---:|---:|---:|---:|---:|
  | a | best of {COVER, raw} x {total/100, /30, /10} by estimate | 0.7242 | 98,929 | 270,196 | 7,621 | 22.8 s |
  | b | a, trained on the no-dictionary plan's stored literals | 0.7612 | 99,907 | 278,113 | 9,164 | 22.0 s |
  | c | a, shortlist of 2 by estimate, each re-planned, smallest plan wins | 0.7191 | 98,929 | 259,004 | 7,621 | 34.6 s |
  | d | c with a shortlist of 3 | **0.7129** | 93,921 | 259,004 | 7,621 | 44.5 s |
  - b is refuted: once dedup has absorbed the repeats, the literals no longer show the dictionary what it needs (invoices lost its dictionary).
  - a's md regression comes from the estimate ignoring dedup: E002's dictionary (COVER total/10) ranks third by estimate but wins on md once re-planned.
- Change (kept: d): `src/repack.rs` `train_dictionaries` returns up to `DICT_SHORTLIST` = 3 candidates (COVER and raw at total/100, /30, /10, clamped 4–110 KB), ranked by `dictionary_cost`. Each is re-planned with and without induction, and the smallest verified plan wins, as before. The two template tests that asserted repack *chooses* templates are split: unit tests in `repack.rs` check the induced plan (one shared template per cluster; layering without a level limit), and integration tests check exactness and size.
- Result: score 0.8157 -> **0.7129**; per corpus vs_dict: md 1.222->1.222, pdf 0.962->0.956, office 1.224->**0.662**, invoices 0.678->0.564, synthetic 0.289->0.289, repo 1.047->1.044; time 20.6 s -> 44.5 s (**2.2x**, repack only: office 10.8 -> 25.4 s, repo 6.7 -> 14.7 s)
  - Office: the dictionary now holds shared XML and the member ids zip records repeat. Records 190,192 -> 57,831 bytes; content Y 170 KB.
  - Invoices: a raw dictionary (essentially the boilerplate) now beats templates (7,621 vs 9,164): statistics beat exact structure when the shared part is small and the 32 B ids of records dominate.
- Verdict: kept. Large score gain and no corpus regresses against E002. The time cost exceeds 2x; a cheaper selection (better estimate, or parallel re-plans) is a follow-up.
- Next gaps: md 1.22 and repo-history 1.04 (tree ids and per-blob framing: office-gap H1; trees not shared across snapshots).

### E004 framing
- Branch / parent: exp/004-framing / E003
- Hypothesis: per-encoding framing is a large share of small stored objects. Naming the dictionary by a 4-byte id prefix instead of 32 bytes, and dropping zstd's own 4-byte dictionary id from frames, shrinks every dictionary encoding and lets more small blobs use the dictionary (source: `docs/research/office-gap.md` H1; zstd.h frame parameters)
- Change:
  - `src/dict.rs`: `DICT_REF_LEN` = 4. `Y` and `lower(tag) 1` encodings carry the prefix; frames are written with `DictIdFlag(false)`.
  - Decoding tries the current dictionary, then every stored content the prefix names, and accepts only output that hashes to the expected id, so a prefix collision can cost a retry but never exactness.
  - `src/history.rs`: gc keeps every object a live prefix names.
  - `src/repack.rs`: the dictionary estimate uses the same framing.
  - New unit test: decoding after the dictionary changes, with a decoy under a same-prefix id.
  - Not done: magicless frames (another 4 B per frame) need zstd's `experimental` feature.
- Result: score 0.7129 -> **0.6643**; per corpus vs_dict: md 1.222->**1.091**, pdf 0.956->0.899, office 0.662->0.600, invoices 0.564->0.493, synthetic 0.289->0.289, repo 1.044->1.027; time 44.5 s -> 44.9 s
  - Bytes: md 93,921->83,920; pdf 23,564->22,156; office 259,004->234,671; invoices 7,621->6,661; synthetic 17,525->17,525; repo 1,347,846->1,325,385
  - md: dictionary encodings now pay off on more small files (Y 274 -> 331; Z 15 -> 1, B 45 -> 2).
- Verdict: kept.
- Next gaps: md 1.09 and repo-history 1.03. Trees are what is left: md 14,226 B for one tree (343 ids), repo-history 203,414 B for 7 trees with mostly the same entries, not shared across snapshots.

### E005 trees-as-content
- Branch / parent: exp/005-trees-as-content / E004
- Hypothesis: successive snapshots' trees are nearly identical, but each is stored whole. Storing a tree's canonical encoding as content lets it be derived from earlier trees by the existing slice, template and dictionary machinery, and re-planned by repack (source: E001/E004 measurements, repo-history trees 203,414 B for 7 near-identical listings; literature in `docs/research/tree-metadata.md`, read during the run; its H1 is the same idea as revlog/git tree deltas)
- Change:
  - `src/dict.rs`: a third object form for trees, `t 2 content[32]`; `decode_object` reads the content. Only trees: derivation records must stay readable without the ledger they make up.
  - `src/repo.rs` `write` stores every new tree this way.
  - `src/history.rs` `reachable` keeps a tree's content live.
  - `src/repack.rs`: `unshare_trees` (at repack start) returns a tree to object form when its content is stored as bytes and no derivation reads it, so a lone tree does not pay the 34 B pointer. `lasting_objects` skips trees stored as content (the re-plan covers them).
  - New law `trees_share_listings_across_snapshots`.
- Variants (all exact and fsck-clean):
  | variant | change | score | md | synthetic | office | repo |
  |---|---|---:|---:|---:|---:|---:|
  | a | every tree stored as content | 0.6572 | 85,852 | 18,260 | 235,822 | 1,153,834 |
  | b | a, unshared trees revert after an applied repack | 0.6520 | 85,817 | 17,525 | 235,788 | 1,153,834 |
  | c | b, the revert runs at repack start, before re-planning | **0.6491** | 83,920 | 17,525 | 234,671 | 1,153,834 |
  - a: the 21 tiny synthetic trees each pay a pointer (+735 B).
  - md and office in a and b: the dictionary differs because the tree's bytes move within the training samples. Diagnostic (not kept): putting tree samples last restores md exactly (83,920) but makes office worse (238,665). The dictionary construction is sensitive to sample order by about ±1–2%; that is a separate open problem.
  - c also fixes the revert being skipped when repack does not apply a re-plan (found by the new test).
- Result: score 0.6643 -> **0.6491**; per corpus vs_dict: md 1.091->1.091, pdf 0.899->0.899, office 0.600->0.600, invoices 0.493->0.493, synthetic 0.289->0.289, repo **1.027->0.894**; time 44.9 s -> 44.6 s
  - repo-history: the 7 trees go from 203,414 B to about 30 KB (7 pointers of 34 B plus content, mostly slices of the first listing); every other corpus is byte-identical to E004.
- Verdict: kept. Every corpus now beats the zstd+dict baseline except md (1.09: one tree of 343 ids, 14 KB).
- Open: dictionary sample-order sensitivity; md's ids.

### E006 abbreviated-ids
- Branch / parent: exp/006-abbreviated-ids / E005
- Hypothesis: a tree in object form is mostly 32 B entry ids that do not compress. Storing each id the store can resolve as an 8-byte prefix shrinks single-snapshot trees by about 60% (source: `docs/research/tree-metadata.md` H3, Meister et al. FAST 2013 page-based codes, 80% of recipe size; git abbreviated object names)
- Change:
  - `src/store.rs`: `Store::ids_with_prefix`. MemStore objects move to a BTreeMap (range query); FsStore reads one fan-out directory.
  - `src/repo.rs`: the ledger's `by_output` becomes a BTreeMap, and `Repo::ids_with_prefix` covers stored and derivable ids.
  - New `src/abbrev.rs`: `abbreviate` writes an entry id as 8 bytes only if it resolves uniquely now. `expand` resolves prefixes and, if later objects made one ambiguous, tries combinations (at most 4,096) and accepts only canonical bytes that hash to the tree id.
  - `src/dict.rs`: tree bodies may be abbreviated (mode flag 0x80); the E004 dictionary lookup uses the prefix query instead of a full scan.
  - Shared trees (stored as content, E005) keep full ids.
  - New unit test: an absent id is kept whole; a same-prefix decoy is resolved by hash.
- Result: score 0.6491 -> **0.6182**; per corpus vs_dict: md 1.091->**0.984**, pdf 0.899->0.854, office 0.600->0.587, invoices 0.493->0.439, synthetic 0.289->0.289, repo 0.894->0.894; time 44.6 s -> 48.5 s
  - Object-form trees: md 14,226->5,999; pdf 2,296->1,172; office 9,631->4,534; invoices 1,109->382. Sizes reproduce exactly on a second run.
- Verdict: kept. Every corpus now beats the zstd+dict baseline.
- Safety note: a prefix collision needs ~2^64 work to create deliberately. If one arises, decoding tries candidates; more than 4,096 combinations in one tree would fail as Corrupt (an error, never wrong data). The bound is unreachable at our scale; it is recorded here, not hidden.
- Open: dictionary sample-order sensitivity (E005); repack time (E003); md's remaining bytes are file content (Y 59.6 KB) against the baseline's 76.9 KB total.

### E007 parallel-repack
- Branch / parent: exp/007-parallel-repack / E006
- Hypothesis: repack's candidate plans (3 dictionaries + none, each with and without induction) are independent and pure, so building them in parallel removes E003's time cost without changing any result (source: E003 time measurement)
- Change: `src/repo.rs` caches use `OnceLock` instead of `OnceCell`, so a `Repo` can be shared read-only across threads. `src/repack.rs` builds the plans in a `std::thread::scope` and compares them in the fixed order used before, so the choice never depends on scheduling. `repack` now needs `S: Store + Sync` (MemStore and FsStore are).
- Result: score 0.6182 -> **0.6182**; every stored size and `by_kind` identical to E006, and identical across two runs; time 48.5 s -> **17.0 s** on 4 cores (office repack 27.1 -> 8.3 s, repo-history 16.3 -> 5.7 s)
  - The speedup depends on the core count; on one core it is the old time.
- Verdict: kept. Same score, E003's flagged time cost removed (now below E002's 20.6 s).

### E008 repeat-dictionary
- Branch / parent: exp/008-repeat-dictionary / E007
- Hypothesis: a raw dictionary built from the content that repeats across samples beats one sampled at even steps, and does not depend on sample order (source: Kuruppu, Puglisi, Zobel, SPIRE 2011, in `docs/research/office-gap.md`: references built from corpus repeats beat sampled ones; E005's order sensitivity)
- Change: `src/repack.rs` adds `repeat_dictionary`: 1 KiB pieces around the seeds found in the most samples, no seed covered twice, most shared last. It is a third candidate construction next to COVER and even-spaced raw, at the same sizes. New unit test: same dictionary for reversed samples; nothing from a single sample.
- Variants (all exact and fsck-clean):
  | variant | shortlist | score | office | repo-history | time |
  |---|---|---:|---:|---:|---:|
  | a | top 3 by estimate (as E003) | 0.6151 | 220,105 | 1,168,263 | 17.0 s |
  | c | top 3 + best of each construction | 0.6151 | 220,105 | 1,168,263 | 20.2 s |
  | d | top 4 + best of each construction | **0.6115** | 214,988 | 1,153,834 | 22.9 s |
  - The repo-history regression in a and c: the 112 KB repeat dictionary ranks first by estimate but re-plans to 1,243,616 B and pushes out COVER-112 KB, E007's winner at 1,152,971 B.
  - Why the estimate misjudges it: content that repeats across files is what slice dedup already removes, so a dictionary of repeats is largely redundant with dedup on repo-history. On office, zip members are not sliced, and it helps.
- Result (d): score 0.6182 -> **0.6115**; per corpus vs_dict: office 0.587->**0.550** (229,574 -> 214,988), every other corpus byte-identical to E007; time 17.0 s -> 22.9 s (1.35x; up to 6 dictionaries, 14 parallel plans). Sizes identical on a second run.
- Verdict: kept.
- Learned: the dictionary estimate ignores dedup, and that keeps costing re-plans. An estimate that discounts content the matcher would slice is the open question (E003b's estimate on a no-dictionary plan's literals failed).

### E009 nested-trees (benchmark change)
- Branch / parent: exp/009-nested-trees / E008
- Change to the measurement, not the kernel: `examples/bench.rs` commits each snapshot as nested trees, one per directory, as a real checkout is stored, instead of one flat tree with `/` replaced by `__`. Decided by the user (tree-metadata H4). **Scores before and after E009 are not comparable**; E009 is the new reference.
- Result, same kernel (E008) on both: score 0.6115 (flat) -> **0.6265** (nested); per corpus vs_dict: md 0.984->1.040, pdf 0.854->0.904, office 0.550->0.559, invoices and synthetic unchanged (no directories), repo 0.894->0.908; time 23.6 s
  - md: 76 trees instead of 1. 48 are tiny and stored canonical (2,376 B), because they are too small to compress and abbreviated ids (E006) apply only inside compressed forms. 28 are stored as content (8,295 B).
  - repo-history: 153 trees per run; unchanged directories are shared by id across snapshots, but each small tree pays its own framing and full 32 B ids.
- Verdict: reference for later experiments. Next: make small trees cheap (abbreviated ids without compression; no pointer form for tiny trees).

### E010 small-trees
- Branch / parent: exp/010-small-trees / E009
- Hypothesis: with nested trees, many trees are too small to compress, so they kept full 32 B ids. An uncompressed abbreviated form recovers most of E006's saving for them (source: E009 measurement; tree-metadata H3)
- Change: `src/dict.rs` adds mode 3, an uncompressed body. `encode_object` now picks the smallest of canonical and {plain, abbreviated} × {zstd, zstd+dictionary, raw}. New unit test: a one-entry tree is stored >20 B smaller and decodes exactly.
- Result: score 0.6265 -> **0.6210**; per corpus vs_dict: md 1.040->1.027, pdf 0.904->0.895, office 0.559->0.559, invoices 0.439->0.439, synthetic 0.289->0.281, repo 0.908->0.905; time 23.7 s
  - md trees (76): 10,671 -> 9,615 B. The flat single tree was 5,999 B, so nesting still costs about 3.6 KB in per-object framing on md.
- Verdict: kept.

### E011 graph-objects (capability)
- Branch / parent: feat/graph-objects / E010
- Hypothesis: a graph layer (cycles, versions, slices) can be added as prolly-tree chunks without changing anything the benchmark measures (source: `docs/plans/2026-10-04-graph-objects.md`; `docs/research/graph-storage.md` G1)
- Change:
  - `src/object.rs`: `G` objects (node chunks and parent chunks); `Node`, `Edge` (target by `node_key`), `Target`, tree entry kind `G`.
  - New `src/graph.rs`: `put_graph` (content-defined chunk boundaries, 4–32 KB), `graph_nodes`, `graph_node`, `graph_slice`, `graph_diff`.
  - `src/history.rs`: reachability keeps chunks and node targets, never edges.
  - `src/repo.rs`: graph chunks are stored as content, like trees (E005).
  - New `tests/graphs.rs`, laws 1–7.
- Result: score 0.6210 -> **0.6210**; every corpus byte-identical to E010. Law 7 measurements are in the design note: 20 versions take 157 KB, 3.3x one version at zstd -19 (one content per version: 1.6x; positional edges: 13x).
- Verdict: kept (capability; score holds). Open: a graph corpus in the benchmark; WebGraph-style reference lists (G3); TerminusDB-style add/remove layers as a competing variant (G2).

### E012 graph-corpus (benchmark change)
- Branch / parent: exp/012-graph-corpus / E011
- Change to the measurement: a seventh corpus, `repo-graph`. It is this repository's dependency graph at each of the 38 commits up to 0ca74e4 (a fixed range): one node per file or module referenced, edges "imports" (Python), "links" (markdown, resolved to a path) and "uses" (Rust `mod`, `use crate::`), from `git grep` per commit.
  - The baseline compresses each version's sorted edge list as one file (zstd with a trained dictionary, as for every corpus). The kernel stores each version with `put_graph` under a tree entry of kind `G`.
  - Exactness: every version's graph, read back through its commit, equals the edge list.
  - **Scores before and after E012 are not comparable**; E012 is the new reference.
- Result, E011 kernel: score **0.5349** over 7 corpora (the six others byte-identical to E010); repo-graph 24,782 B vs baseline 113,441 B (0.218); input 2.94 MB, about 77 KB per version; time 25.6 s
  - Like the other history corpora, the baseline compresses each version alone, which favours the kernel.
  - Unscored comparison within the kernel: the same edge lists stored as plain files take 14,513 B, so graph form costs 1.7x for chunk-level lookup, diff and slicing (as law 7 measured on synthetic graphs).
  - Breakdown: graph chunks g 54 objects 5,795 B plus their content (Y/Z/d) ~15 KB; 38 commits 2,742 B (11%).
- Verdict: reference for later experiments. Next: WebGraph-style reference lists (graph-storage G3).

### E013 reference-adjacency (refuted, probe only)
- Branch / parent: none (no kernel change) / E012
- Hypothesis: WebGraph-style reference lists (copy a similar previous node's edges by bitmask, plus extras; window 7) shrink graph chunks (source: `docs/research/graph-storage.md` G3; Boldi–Vigna, 4.17 -> 3.08 bits/link on WebBase)
- Probe (temporary example, not committed; source in the session scratchpad `ref_probe.rs`): the final repo-graph version, 634 nodes and 1,121 edges, zstd -3 per chunk:
  | nodes per chunk | canonical | reference lists | front-coded labels |
  |---|---:|---:|---:|
  | 64 | 13,173 B | 13,303 B | 12,465 B |
  | 256 | 11,088 B | 11,178 B | 10,359 B |
  | whole graph | 10,301 B | 10,330 B | 9,477 B |
- Verdict: refuted for this corpus. With 1.8 edges per node, neighbours' edge lists rarely overlap; WebGraph relies on dozens of links per node with URL-order locality.
- Learned:
  - The bytes are the edges' 8-byte target keys: 1,121 x 8 B = 8,968 B of random data, nearly all of the 9,477 B left after front coding.
  - **Correction (E014):** wrong. The 1,121 edges have only 100 distinct targets, so the keys compress to 1,788 B (17%). Node labels are 56% (5,719 B).
  - Front-coded labels save 5–8%.
  - ~~The next lever is key width~~ (see the correction above and E014).

### E014 short-edge-keys (not applicable, probe only)
- Branch / parent: none (no kernel change) / E013
- Hypothesis as proposed: store each edge's target key as its shortest prefix unique within the chunk (stored form only, as E006 did for tree ids).
- Why it cannot work as stated: targets mostly live in other chunks, so a chunk-local prefix cannot be expanded back to the full key without reading the whole graph or keeping a second, key-sorted index. E006 worked because tree ids resolve against the store.
- Workable variant probed instead: a per-chunk table of distinct target keys, with edges as u16 indices (with front-coded labels): 12,718 / 10,427 / 9,591 B at 64 / 256 / all nodes per chunk, against 12,465 / 10,359 / 9,477 B without the table.
- Decomposition of the final repo-graph version (zstd -3, each part compressed alone): node labels 5,719 B (56% of 10,301), edge keys 1,788 B (17%; 1,121 edges, only 100 distinct targets), edge counts 378 B, edge labels 108 B, the rest length fields and framing.
- Verdict: not applicable. Keys are not where the bytes are, and zstd already compresses repeated keys. Node labels are; front coding them saves 5–8% of a chunk (E013 probe).

### E015 magicless-frames
- Branch / parent: exp/015-magicless / E012
- Hypothesis: nested trees multiplied small stored objects (md: 333 files and 76 trees). Each zstd frame repeats a 4-byte magic number and a content-size field that the surrounding encoding and the id check make redundant, so dropping them recovers per-object overhead (source: `docs/research/office-gap.md` H1, zstd.h frame parameters; magicless frames left untried in E004)
- Change:
  - `Cargo.toml`: zstd `experimental` feature (exposes the frame-format parameter).
  - `src/dict.rs`: `zstd_frame`, `framer` and `unzstd_frame` write and read every stored frame (Z, Y, compressed objects, the dictionary) as magicless, with no content size, checksum or dictionary id.
  - `src/repack.rs`: the dictionary estimate uses the same framing, with one compressor per dictionary.
  - Decoding is unchanged in what it guarantees: output is checked against its id.
- Result: score 0.5349 -> **0.5270**; per corpus vs_dict: md 1.027->1.006, pdf 0.895->0.882, office 0.559->0.547, invoices 0.439->0.427, synthetic 0.281->0.279, repo-history 0.905->0.902, repo-graph 0.218->0.216; time 25.6 s -> 25.2 s
  - Bytes: md 78,935->77,352; office 218,801->214,168; repo-history 1,168,467->1,164,322.
  - A first version loaded the dictionary for every sample in the estimate (29.7 s); reusing one compressor per dictionary fixed it, with identical sizes.
- Verdict: kept. md is still just above the baseline (1.006): Y 333 objects 58,025 B, trees 76 objects 9,479 B, dictionary 9,807 B.

### E016 solid-groups
- Branch / parent: exp/016-solid-groups / E015
- Hypothesis: small contents compressed one by one lose their neighbours' context and pay a frame each. Compressing similar small contents together in groups of up to 64 KB beats per-content compression with a shared dictionary, even after a short entry per member (source: `docs/research/office-gap.md` H5; Shilane et al. FAST 2012 compression regions). Probe first (temporary, not committed): md 76.2 KB per file with dictionary -> 69.2 KB in 64 KB groups including 13 B per member; office zips -5 to -11%.
- Change:
  - New `src/group.rs`. A member's stored entry is `S group[8] start len` (varints). A group is ordinary content, the concatenation of its members, stored plain (with the dictionary). Members are read through a prefix lookup, and each read is checked against the member's id, so a shared prefix costs a retry, not exactness.
  - `group_small` runs at the end of each re-plan, so repack compares plans with grouping applied. It sorts small stored contents by bytes, packs groups up to `GROUP_SIZE` = 64 KiB, and writes a group only if it and its entries are smaller than the members' encodings.
  - `src/dict.rs`: `S` is a plain encoding; `needs` covers dictionaries and groups.
  - `src/history.rs`: gc keeps every object a live encoding's prefix names.
  - Tests:
    - New unit tests: varints; grouped members read back past a decoy, gc keeps the group, fsck flags exactly the forged decoy.
    - `revisions_repack_into_templates` asserted that repack *chooses* templates. With grouping, three near-identical revisions per 64 KB group compress against each other, and the re-plan without templates wins (21 revisions in 14,893 B, 1.56x one revision at zstd -19). The test now asserts that outcome: under 2x one revision, whichever plan wins.
- Result: score 0.5270 -> **0.4851**; per corpus vs_dict: md 1.006->**0.909**, pdf 0.882->0.834, office 0.547->**0.442**, invoices 0.427->0.393, synthetic 0.279->0.279, repo-history 0.902->0.881, repo-graph 0.216->0.196; time 25.2 s -> 28.5 s (1.13x). Sizes identical on a second run.
  - md: 333 members (4,481 B of entries), 3 groups (52,526 B); the dictionary shrank 9,807 -> 2,824 B, since the groups carry the shared context.
  - office: 656 members (8,993 B), 21 groups (95,054 B).
- Verdict: kept. Every corpus now beats the zstd+dict baseline.
- Known limits:
  - Reading a member decompresses its group (up to 64 KiB); the evaluator's read weight does not price decompression yet.
  - A group keeps the bytes of members that die until the next repack regroups.

### E017 seed-similarity groups
- Branch / parent: exp/017-seed-groups / E016
- Hypothesis: grouping contents by resemblance (shared content-defined seeds, MinHash-like) instead of by byte order puts more redundancy in each group (source: E016's byte-order grouping; Broder-style resemblance via the matcher's anchors; literature in `docs/research/semantic-grouping.md`, read in parallel)
- Change: `src/group.rs`:
  - `similar_groups`: a greedy proposer. It starts from the first unplaced content in byte order and adds the unplaced content sharing the most seeds with the group so far, using an inverted seed index.
  - `group_small` plans every proposer's grouping without writing (`plan_group`) and writes the one that saves most. This keeps byte order (`byte_order_groups`) as a competitor, and later proposers (embeddings, LLM) plug in the same way.
- Variants:
  | variant | score | md | pdf | office | invoices | repo-history | repo-graph |
  |---|---:|---:|---:|---:|---:|---:|---:|
  | a: seed proposer only | 0.4856 | 68,911 | 20,601 | 173,282 | 5,454 | 1,119,910 | 22,313 |
  | b: best of {byte order, seeds} | **0.4830** | 68,911 | 20,547 | 172,886 | 5,309 | 1,119,910 | 22,195 |
  - Seeds win on md (-1.4%) and repo-history (-1.5%); byte order wins on invoices, pdf, office and repo-graph, where files with the same header already sort together.
- Result (b): score 0.4851 -> **0.4830**; per corpus vs_dict: md 0.909->0.896, repo-history 0.881->0.868, others unchanged; time 28.5 s -> 23.5 s (measured twice; cause not established). Sizes identical on a second run.
- Verdict: kept.

### E018 marginal-cost groups
- Branch / parent: exp/018-marginal-cost / E017
- Hypothesis: the compression-native resemblance, the bytes a content adds to a group as measured by the kernel's own zstd and dictionary, predicts group compressibility better than seed overlap alone (source: `docs/research/semantic-grouping.md` H2; Cilibrasi and Vitanyi's NCD; DeepSketch, FAST 2022: sketches miss a usable reference for 35.7% of blocks, and a hash trained on measured gains gives +21% data reduction)
- Change: `src/group.rs` adds a third proposer, `similar_groups(items, Some(dict))`. Seed scores shortlist `SHORTLIST` = 8 candidates per step, and the next member maximises `C(x) - (C(G x) - C(G))` with the kernel's zstd framing and dictionary. The best of the three proposers is written, as in E017.
- Result: score 0.4830 -> **0.4798**; per corpus vs_dict: md 0.896->0.894, office 0.442->0.437, invoices 0.393->0.384, synthetic 0.2787->0.2786, repo-history 0.868->0.859, pdf and repo-graph unchanged; time 23.5 s -> 28.1 s (1.2x). Sizes identical on a second run.
- Verdict: kept. Measured compression beats resemblance proxies wherever both are tried, as the literature predicted.

### E019 embedding-proposer
- Branch / parent: exp/019-embedding-proposer / E018
- Hypothesis: a modern semantic signal, sentence embeddings, proposes groups that compress better than byte order, seeds or measured cost. The literature expected it to be weakest, since embeddings group by topic and zstd needs shared byte runs, but no paper measured storage bytes (source: `docs/research/semantic-grouping.md` H4; SemDeDup, D4, Ferragina and Manzini)
- Change:
  - New `packages/kernel/semantic/embed.py`, an offline tool. It embeds every content the benchmark stores (md files, office zip members, repo-history files; 1,494 contents) with a pinned model: all-MiniLM-L6-v2 @ 1110a243, ONNX sha256 6fd5d72f…, 256 tokens. It writes `semantic/order.txt`, a greedy cosine nearest-neighbour chain per corpus. Its ids match the kernel's (checked against `ikam put`).
  - Kernel: `Repo::proposed_order` is a model-agnostic hook carried into re-plans. `group.rs` packs groups in that order as a fourth proposer, kept only if it saves most.
  - `examples/bench.rs` reads the committed `order.txt`; no model runs in the benchmark.
- Per-proposer bytes saved by grouping (diagnostic, every candidate plan; showing the plan without a dictionary):
  | corpus | byte order | seeds | measured cost | embeddings |
  |---|---:|---:|---:|---:|
  | md | 46,341 | 47,117 | **47,341** | 46,905 |
  | office | 158,750 | 163,925 | **165,921** | 158,735 |
  | repo-history | 124,593 | 147,897 | **156,733** | 153,483 |
  - Embeddings beat lexical seeds on repo-history in every plan and on md in most, contrary to the literature's ranking. Measured cost still wins nearly every plan, and on office embeddings are about equal to byte order.
- Variants:
  - a (kept): the embedding order as a proposer.
  - b: the embedding order shortlists candidates and measured cost picks (DeepSketch-style). Score 0.4763, but **not a semantic gain**: it improved only invoices (5,191 -> 4,941) and repo-graph, which have no embeddings. The fallback ranked their items in byte order, making b also a "byte-order shortlist + measured cost" proposer. md and repo-history did not improve. Time +30%. That proposer is tested on its own in E020.
- Result (a): score 0.4798 -> **0.4797**; md 68,706 -> 68,625 (-81 B), every other corpus unchanged; time 28.1 s -> 30.3 s. Sizes identical on a second run.
- Verdict: kept as a capability (an offline, reproducible semantic proposer and the hook an LLM proposer would use), with a **neutral storage effect** on this benchmark. Embeddings are informative but are dominated by measured compression here.

### E020 byte-order-cost groups
- Branch / parent: exp/020-byte-order-cost / E019
- Hypothesis: measured cost helps whichever proposal shortlists candidates. A proposer that grows groups in byte order but adds, from the next 8 unplaced contents, the one the group compresses best wins where byte order already wins (invoices, repo-graph) (source: E019 variant b, whose gain came from this fallback and not from embeddings; `docs/research/semantic-grouping.md` H2)
- Change: `src/group.rs`: `similar_groups` takes an optional rank order that shortlists candidates and starts groups (byte order here), and a fifth proposer uses it with measured cost. The E019 embedding proposer is unchanged.
- Result: score 0.4797 -> **0.4762**; per corpus vs_dict: invoices 0.384->0.366 (5,191 -> 4,941), repo-graph 0.1957->0.1954, every other corpus byte-identical to E019 (md keeps the embedding proposer's 81 B); time 30.3 s -> 34.0 s (1.12x). Sizes identical on a second run.
- Verdict: kept. Across E017–E020, measured compression is the deciding signal, and resemblance signals (byte order, seeds, embeddings) are only useful as shortlists.

### E021 generated-corpus (benchmark change)
- Branch / parent: exp/021-generated-corpus / E020
- Change to the measurement: an eighth corpus, `generated`. It is data that is the output of a small integer-only procedure, where a reconstruction function (a program) can be far smaller than any compression of its output (source: `docs/plans/2026-10-06-system-architecture.md`, functions as a library in the graph; the KoLMogorov Test: programs beat compressors on generated data, not on prose). One snapshot of five files, 688,414 B:
  - `metrics.csv`: 4,000 rows of computed columns and a running total.
  - `multiplication.txt`: a 99 x 99 table.
  - `server.log`: 6,000 lines with sequential timestamps and ids, and LCG-chosen paths, statuses and latencies.
  - `mandelbrot.pgm`: 320 x 240, fixed point with 12 fractional bits.
  - `primes.txt`: the first 20,000 primes.
  - **Scores before and after E021 are not comparable**; E021 is the new reference.
- Result, E020 kernel: score **0.5225** over 8 corpora; the seven others byte-identical to E020. generated: 175,169 B vs baseline 175,017 B (1.001), so the kernel does no better than zstd here, as expected without functions.
- Verdict: reference for the function-library experiments.

### E022 interpreter-programs
- Branch / parent: exp/022-interpreter-programs / E021
- Hypothesis: data produced by a procedure is stored far smaller as a short program over one shared, stored interpreter than as compressed bytes, once the interpreter, the programs and the read cost (measured fuel) are all paid for (source: `docs/plans/2026-10-06-system-architecture.md`, functions in the graph; the KoLMogorov Test; minimum description length as in DreamCoder, Stitch and Brevis)
- Change:
  - New `packages/kernel/functions/`:
    - `interp/` is an integer stack machine, `no_std`, no floats, no imports; `interp.wasm` is 6,942 B, stored as 3,133 B.
    - `asm.py` is the assembler.
    - `programs/` holds five **hand-written** programs (a baseline proposer, standing in for the LLM writer of step 4): multiplication 50 B, metrics 136 B, server_log 299 B, mandelbrot 185 B, primes 113 B (a sieve; a first trial-division version needed 41.9 G fuel and was replaced).
    - `proposals.txt` holds each program's output id, size and decode work, written by `examples/programs.rs`.
  - Kernel:
    - `wasm::run` reports the fuel spent, and `func::run` returns decode work = bytes produced + fuel / `FUEL_PER_WORK` (1,000). The evaluator now prices WASM fuel, not only output bytes.
    - The fuel cap rises from 2^30 to 2^38. It is a hard bound on any read (~30 s at measured speeds); raising it only lets more calls succeed.
    - `Repo::evaluate` prices a candidate without recording it.
  - `examples/bench.rs` offers a priced program when its output is a file of the corpus and its decode work could pay at the default read weight. The kernel verifies it; repack keeps it only if it saves bytes.
- Measured:
  - wasmi charges ~1,300 fuel per interpreted op; the same module under wasmtime executes ~85 instructions per op.
  - Mandelbrot needs 255 G fuel (~28 s): priced at ~255 KB-equivalent read cost for a 77 KB output, it is never offered and stays stored. The other four are priced at 211–11,202 byte-equivalents.
- Result: score 0.5225 -> **0.3770**; generated 175,169 -> **12,851 B** (vs_dict 1.001 -> 0.073); the seven other corpora byte-identical to E021; time 42.7 s -> 45.3 s. Sizes identical on a second run. What generated now stores: the interpreter and the Mandelbrot image (Z, 12,201 B), 4 derivation records (408 B), the grouped programs (62 B of entries), tree and commit.
- Verdict: kept, with caveats:
  - The programs are hand-written. This shows what the mechanism can do, not what an automatic proposer finds. The LLM writer (step 4) is the real test.
  - The score is a geometric mean, so one corpus improving 14x moves it a lot (0.5225 -> 0.3770). Per-corpus ratios are the honest view.

### E023 compiled-programs
- Branch / parent: exp/023-compiled-programs / E022
- Hypothesis: interpreted programs are slow to read under wasmi (~1,300 fuel per op), which kept Mandelbrot from paying. Compiling each program to its own WASM module cuts its decode work by orders of magnitude, so more of the generated corpus can be stored as functions (source: E022 measurements)
- Change:
  - New `functions/compile.py` turns program bytecode into WAT: a dispatch loop (`br_table`) over basic blocks, the operand stack on WASM's value stack, registers as locals, the scratch array and output in linear memory, plus runtime helpers. It refuses programs whose stack is not empty at a block boundary; those stay interpreted.
  - `examples/programs.rs` assembles each WAT with the `wat` crate and prices both forms. `proposals.txt` lines are now `function argument output-id bytes work`.
  - The `examples/bench.rs` proposer offers, per corpus file, the cheapest form: function bytes (the interpreter's shared by its users) + argument bytes + read weight x work, if below the output size. The kernel verifies it; repack keeps it only if it pays.
  - A first compile emitted string addresses relative to the data segment instead of its base, so every output differed (newlines became zero bytes). The id comparison caught it before use; after the fix all five compiled outputs equal the interpreted ones byte for byte.
- Decode work, interpreted -> compiled: mandelbrot 255,193,789 -> 580,478 (440x); primes 11,202,154 -> 228,638 (49x); server_log 910,458 -> 327,045; metrics 378,822 -> 121,809; multiplication 211,358 -> 53,915. Compiled modules are 2.4–2.9 KB each.
- Result: score 0.3770 -> **0.3444**; generated 12,851 -> **6,241 B** (vs_dict 0.073 -> 0.036; Mandelbrot is now a function too); the other seven corpora byte-identical; time 45.3 s -> 43.9 s. Sizes identical on two runs.
- Verdict: kept. The programs are still hand-written (E022's caveat), and the score's change again comes from one corpus.

### E024 function-library (capability) and fuel cap restored
- Branch / parent: exp/024-function-library / E023
- Hypothesis: the reconstruction functions in use can be kept as a graph in the semantic layer (functions, their applications, measured savings and decode work) at negligible cost (source: `docs/plans/2026-10-06-system-architecture.md`, functions as a library in the graph)
- Change:
  - New `src/library.rs`: `Repo::function_library` rebuilds, from the ledger, a graph with nodes per non-builtin function, application (derivation record) and content read or produced. Edges are "function", "argument" and "output (saves N B, work W)". It points the ref `meta/functions` at the graph; same ledger, same graph.
  - `repack` calls it after re-planning, whether or not the re-plan applied.
  - New test `repack_publishes_the_function_library_graph`.
  - **Correction to E022:** the fuel cap returns from 2^38 to 2^30. At 2^38, a module that never stops ran **572 s** (the sandbox test: `tests/exec.rs` took 573 s); at 2^30 it stops in 2.2 s. E022's "about 30 s" was an extrapolation and wrong for tight loops. Compiled programs (E023) all fit under 2^30 (largest: Mandelbrot, ~0.58 G fuel). Interpreted Mandelbrot and primes no longer fit and drop out of `proposals.txt`.
- Result: score 0.3444 -> **0.3307**; generated 6,241 -> **4,512 B** (vs_dict 0.036 -> 0.026); other corpora byte-identical; time 43.9 s -> 44.9 s. Sizes identical on two runs.
  - **Attribution:** the gain is not from the library graph, which adds bytes (a 34 B pointer plus its content). It comes from the regenerated proposals. With interpreted primes and Mandelbrot gone, the proposer splits the interpreter's size over 3 files instead of 5, prefers compiled modules for all five files, and the interpreter is no longer stored. The five modules share their runtime code and compress well in one group (5 members, 64 B of entries).
  - The proposer's cost estimate (raw module size, not grouped compression) is crude; the kernel's evaluator is what keeps choices sound.
- Verdict: kept (capability; and the fuel cap correction).

### E025 llm-programs
- Branch / parent: exp/025-llm-programs / E024
- Hypothesis: an LLM can write stack-machine programs that reproduce the generated corpus's files exactly, and storing them through the existing pricing and proposer pipeline beats the hand-written programs (source: `docs/plans/2026-10-06-system-architecture.md`, the LLM writes reconstruction functions; E022's caveat that its programs were hand-written; the KoLMogorov Test)
- Change:
  - `functions/write_programs.py` (written before this run, unchanged by it) ran once per file. Claude (`claude-opus-5-5`, effort high, server-side fallbacks on) saw the instruction set, one non-target example, and the file's size, first 3,000 and last 600 characters (binary: first 512 and last 128 bytes as hex). It had up to 3 attempts per file, each verified byte for byte through `ikam apply`, with the first mismatch fed back.
  - New `functions/programs/llm_{multiplication,metrics,primes}.{asm,bin,wat,wasm}`, `functions/llm_runs.jsonl` (9 calls), and `functions/proposals.txt` regenerated by `examples/programs.rs` (hand-written and LLM lines together). No kernel or benchmark code changed.
- Programs (bytecode bytes; compiled decode work; attempts):
  | file | LLM | hand-written (E022/E023) |
  |---|---|---|
  | multiplication.txt | 50 B, work 53,915, 1 attempt | 50 B, 53,915 (**byte-identical bytecode**) |
  | metrics.csv | 122 B, work 119,247, 1 attempt | 136 B, 121,809 |
  | primes.txt | 74 B, work 211,286, 1 attempt (sieve) | 113 B, 228,638 |
  | server.log | failed, 3 attempts (687–705 B programs, all mismatched) | 299 B, 327,045 |
  | mandelbrot.pgm | failed, 3 attempts (18 B, 0 B (rejected), 115 B), all served by `claude-opus-4-8` via fallback | 185 B, 580,478 |
  - Compiled modules: llm 2,445–2,538 B vs hand 2,445–2,609 B. Interpreted llm_primes, like hand primes, exceeds the 2^30 fuel cap.
  - The failures fit what the prompt showed: server.log's paths, statuses and latencies come from an LCG, and mandelbrot's fixed-point parameters must be inferred from 512 hex bytes. Neither is determined by the samples. *[inference: the mismatch feedback was not logged]*
  - Cost: 9 calls, 41,629 input + 12,735 output tokens, **$0.42** as logged (every call priced at Opus 5.5 rates). Repriced at Opus 4.8's $5/$25 for the 3 fallback calls: $0.44. Check the workspace usage page for the billed amount.
- Result (committed proposals): score 0.3307 -> **0.3541** (worse); generated 4,512 -> **7,793 B** (vs_dict 0.0258 -> 0.0445); md 0.8925, pdf 0.8338, office 0.4370, invoices 0.3655, synthetic-history 0.2786, repo-history 0.8586, repo-graph 0.1954, all byte-identical to E024; time 37.4 s (E024 proposals on the same machine: 37.6 s). Sizes identical on two runs. E024's proposals re-measured on this build give 0.3307 exactly.
  - **Attribution:** the regression comes from the regenerated proposals, not from the LLM programs. `bench.rs` divides the interpreter's size by the number of proposal *lines* that use it. With LLM lines added, 5 interpreted lines cover only 3 distinct files. The interpreter looks 5-way shared, so metrics, multiplication and server_log switch to interpreted form, and the interpreter is stored again (Z 3,157 -> 6,777 B; derivation records 345 -> 714 B; the module group shrinks).
  - Diagnostic, not committed: a proposals file in which LLM programs replace the hand-written ones for the same three files (hand server_log and mandelbrot kept) gives generated **4,444 B** (-68 B vs E024, vs_dict 0.0254) and score **0.3301** (one run). Only this number measures what the LLM programs are worth: a small gain, from smaller compiled modules for metrics and primes.
- Also measured: `examples/programs.rs` prices every program in one shared repo, so a module put after similar modules is stored deduplicated and reading it back adds its size to the work. mandelbrot.wasm's work is 580,478 when priced alone and 583,116 (+2,638 = its module size) after the llm_* modules. Pricing depends on file order. Here the effect is small and only changes the proposer's hint.
- Verdict: **discarded** as committed (the score worsens). Results: Claude reproduced 3 of 5 files exactly on the first attempt, for $0.42. Its programs are equal to or smaller than the hand-written ones. It could not infer the LCG or the fixed-point parameters from samples. The proposer's cost estimate, not program quality, decides the score here. Fixing it (count distinct files per shared function; account for grouping) is the next hypothesis. Not merged into feature/rust-kernel.

### E026 grouping-aware-proposer
- Branch / parent: exp/026-grouping-aware-proposer / E025 (E025's proposals, with hand-written and LLM forms side by side, are the input that broke the old estimate; E025 itself was not merged)
- Hypothesis: pricing the proposer's choices jointly, as repack stores them, stops the interpreter from being counted as shared by duplicate proposal lines and lets similar modules share their cost. This recovers E025's loss and lets the LLM programs pay (source: E025 measurement: 5 interpreted lines over 3 files made the interpreter look 5-way shared; generated 4,512 -> 7,793 B; with LLM programs swapped in by hand, 4,444 B)
- Change: `examples/bench.rs` `propose_programs`. The per-line cost (function size / proposal lines using it + argument size + w x work) is replaced by a joint total: zstd level 3 (the kernel's group level) of the distinct function and argument contents chosen, concatenated as one group, + `DEFAULT_READ_WEIGHT` x total work + zstd of each file left stored. Coordinate descent: for each file, pick the form (or none) that lowers the total; repeat until nothing changes. No kernel code changed.
- Result: score 0.3307 (E024, the current best) -> **0.3302**; vs E025 0.3541. generated 4,512 -> **4,454 B** (vs_dict 0.0258 -> 0.0254). md 0.8925, pdf 0.8338, office 0.4370, invoices 0.3655, synthetic-history 0.2786, repo-history 0.8586, repo-graph 0.1954, all byte-identical to E024. Time 37.9 s (E024 proposals on the same machine: 37.6 s). Sizes identical on two runs.
  - Chosen: compiled modules for all five files; the hand-written forms for metrics, multiplication (byte-identical to the LLM one), server_log and mandelbrot, and **llm_primes** for primes. The interpreter is no longer stored (Z 3,157 -> 3,100 B; the module group 732 -> 731 B).
  - **Attribution:** the 58 B gain is from an LLM-written program (llm_primes, 74 B of bytecode vs 113 B). The proposer change is what lets it be chosen without the E025 regression. On its own the proposer change is neutral: with E024's proposals it picks E024's forms and gives exactly 4,512 B (diagnostic run, generated corpus only).
  - The estimate is approximate. It chose the hand-written metrics module, while E025's hand-picked diagnostic with llm_metrics measured 4,444 B (10 B less). The kernel's evaluator still decides what is kept.
  - Limitation: coordinate descent changes one file at a time, so it cannot find a state that pays only when several files switch together (e.g. all files moving to the shared interpreter). Exhaustive search was not needed for 5 files with 2–3 options each, but it was not tried.
- Verdict: **kept** (the score improves over E024; neutral on E024's input). Merging brings E025's LLM programs and proposals into feature/rust-kernel, since this result depends on them. The gain is small and from one corpus (the geometric mean moves 0.15%).

### E027 order-independent-pricing
- Branch / parent: exp/027-order-independent-pricing / E026
- Hypothesis: pricing each program form in its own fresh store removes an order-dependent bias in `proposals.txt`. In a shared store, a module put after similar modules is stored deduplicated, and reading it back adds its size to the measured decode work (source: E025 measurement: mandelbrot.wasm 580,478 alone, 583,116 after the llm_* modules)
- Change: `examples/programs.rs` builds a new `Repo` for each priced form (function, plus the program as argument when interpreted) instead of one repo for all. `functions/proposals.txt` regenerated. No kernel or benchmark code changed.
- Measured:
  - The bias was systematic, not only an E025 artifact. The compiled modules share their runtime code, so every module after the first was charged about its own size. Compiled work, shared store -> fresh store: metrics 121,809 -> 119,263; multiplication 53,915 -> 51,470; primes 228,638 -> 226,029; server_log 327,045 -> 324,184; llm_multiplication 53,915 -> 51,470; llm_primes 211,286 -> 208,772; mandelbrot 583,116 -> 580,478. Unchanged: llm_metrics, 119,247 (first in order); interpreted forms (the interpreter was always put first).
  - **Correction:** the compiled decode work reported in E023 (and used in E024–E026 proposals) was inflated by 2.4–2.9 K for every module but the first. Only mandelbrot's 580,478 was right, because it was the first module. At `DEFAULT_READ_WEIGHT` 0.001 this was at most ~3 B of estimated cost per file.
  - Reversing the file order gives the same `proposals.txt` lines (checked once, with a temporary edit).
- Result: score 0.3302 -> **0.3302**; every corpus byte-identical to E026, generated included (4,454 B, the same forms chosen): md 0.8925, pdf 0.8338, office 0.4370, invoices 0.3655, synthetic-history 0.2786, repo-history 0.8586, repo-graph 0.1954, generated 0.0254; time 37.9 s (E026: 37.9 s). Sizes identical on two runs.
- Verdict: **kept** as a measurement correction. Neutral on storage: at the default read weight the bias was too small to change any choice. It would matter at higher read weights, or with more similar modules priced in one run.

### E028 range-reads
- Branch / parent: exp/028-range-reads / E027
- Hypothesis: a read can decode only what it needs, with every leaf it decodes still verified against its own id. Mapping a byte range backwards through `concat` and `fill` plans, and decoding a group's zstd frame only up to the member's end (on average half the group), lowers read cost with storage unchanged (source: `docs/plans/2026-10-06-system-architecture.md`, "Partial reads", which predicted the half-group figure; zstd frames decode sequentially, so a prefix costs only its length)
- Change:
  - New `src/range.rs`: `Repo::read_range(id, start, len) -> (bytes, work)`.
    - Through `concat` it reads only the overlapping arguments, recursively. `Arg::Range` carries its length; an `Arg::Whole` before the range is read to learn its length.
    - Through `fill` it decodes the template and fillers, then produces only the overlapping pieces (`template::pieces`, which `fill` now uses too).
    - A stored leaf, `deflate-pack` or WASM is read whole, verified, and sliced. Out-of-range or overflowing requests are errors.
    - Soundness: leaves are verified by id. A walked plan relies on its derivation record having been verified when recorded (and by fsck); the whole output's id is not re-checked.
  - `src/dict.rs`: `unzstd_prefix` decodes at most `limit` bytes of a frame, and `decode_prefix` does the same for a plain "B"/"Z"/"Y" encoding, under a caller's check. A per-thread `decompressed_bytes()` counter (measurement only) counts bytes decompressed from frames.
  - `src/group.rs` `read_member`: decodes the group only up to the member's end and verifies the member by its own id (as before; the group's id was never what verified a member). This applies to **every** member read, whole or range.
  - `examples/bench.rs` reports `read_whole` and `read_4k` per corpus: over every distinct file, bytes decompressed + bytes produced by functions, for a whole read and for a 4 KiB read from the file's middle. Every range read must equal its slice, or the exactness gate fails. Graph corpora are skipped (0).
  - Test `range_reads_equal_slices_of_whole_reads` (`tests/templates.rs`): windows over revisions and invoices, after ingest (slices) and after repack (templates, groups), equal slices of whole reads; at least one read through a plan costs less than the whole read; out-of-range reads are refused.
  - `src/ingest.rs`: a struct-initializer form for `Cx`, required by clippy once `Cx.stack` became crate-visible. No behaviour change.
- Read cost (decoded bytes; deterministic). "Full group" is a diagnostic run with the prefix limit removed (one run, reverted):
  | corpus | whole, full group | whole, E028 | 4 KiB range, E028 | 4 KiB / whole |
  |---|---:|---:|---:|---:|
  | md | 18,714,864 | 9,265,537 (0.495x) | 9,265,537 | 1.000 |
  | pdf | 2,386,018 | 1,259,030 (0.528x) | 1,259,030 | 1.000 |
  | office | 239,158,160 | 162,904,289 (0.681x) | 162,904,289 | 1.000 |
  | invoices | 1,565,130 | 825,330 (0.527x) | 825,330 | 1.000 |
  | synthetic-history | 1,743,558 | 1,738,154 (0.997x) | 1,012,279 | 0.582 |
  | repo-history | 52,233,071 | 33,631,273 (0.644x) | 31,206,091 | 0.928 |
  | generated | 1,349,187 | 1,322,701 (0.980x) | 1,322,701 | 1.000 |
  - **Attribution:** almost all of the gain is from prefix decoding of groups, and it applies to whole reads too (0.50–0.68x on md, pdf, office, invoices, repo-history). Range walking through plans helps only where files are plans: synthetic-history (0.58x of a whole read) and repo-history (0.93x). Elsewhere a 4 KiB read costs exactly as much as a whole read, because the file is a group member, a zip container or a WASM output, and the cost is decoding the leaf or container, not walking a plan.
  - Reads still decode far more than they return: md's whole reads decode 9.3 MB to return its files (input 178,741 B over all snapshots), because each member read decodes its group from the start (up to 64 KiB) and office reads rebuild zips member by member.
- Result: score 0.3302 -> **0.3302**; every corpus byte-identical to E027: md 0.8925, pdf 0.8338, office 0.4370, invoices 0.3655, synthetic-history 0.2786, repo-history 0.8586, repo-graph 0.1954, generated 0.0254; time 36.3–36.7 s (E027: 37.9 s; the full-group diagnostic: 37.3 s). Sizes and read costs identical on three runs (the third after the clippy fix).
- Verdict: **kept** (score holds while adding a capability). Range reads through plans and half-cost member reads, both still verified per leaf. Not done: container ranges (zip members), WASM range maps, and per-segment reads inside a group (seekable blocks), which the read costs above point to as the remaining cost.

### E030 zstd-level
- Branch / parent: exp/030-zstd-level / E028
- Hypothesis: most stored bytes are zstd frames, all written at level 3, so a higher level shrinks the store at almost no read cost (zstd decodes about as fast at any level). Comparing plans at level 3 and compressing only the kept plan at the high level keeps most of the gain at a fraction of the time (source: measurement. zstd frames (Y/Z) are 47–96% of stored bytes per corpus in E028; on whole-file samples zstd-19 is 0.84x of zstd-3 on the repo's markdown, 0.82x on its Rust source, 0.65x on server.log, 0.59x on metrics.csv, with xz -9e and brotli-11 smaller still)
- Change:
  - `src/dict.rs`: `MEASURE_LEVEL` (3) for estimates (group growth, dictionary scoring, trial plans) and for what ingest writes; `STORE_LEVEL` (19) for what repack keeps. `framer` stays at the measuring level; `zstd_frame` takes a level; encoders use the repo's `level`.
  - `src/repo.rs`: `Repo.level`, `MEASURE_LEVEL` by default.
  - `src/repack.rs`: `Repo::recompress` re-encodes every plain frame and compressible object at `STORE_LEVEL`, checking decoded content against its id first, and keeps an encoding only if it is smaller. Repack compares plans at level 3, then recompresses the kept plan before comparing it with the current store. Objects repack re-encodes afterwards also use `STORE_LEVEL`. The dictionary itself is only ever re-encoded as "B" or "Z" (decoding never chains).
  - Test `recompress_shrinks_and_keeps_the_dictionary_plain` (fails with the dictionary guard removed: checked once).
  - A first version without that guard re-encoded the dictionary against itself. The exactness gate failed on md (`Corrupt` on read) before any score was recorded.
- Variant a (diagnostic; committed as wip `cfdf03c`, superseded): level 19 for **everything**, including every trial plan and measurement. One run: score 0.2851; time 37.3 s -> **1,175.7 s (32x)**, e.g. office repack 12.4 s -> 426.6 s, repo-history 15.7 s -> 566.8 s.
- Result (kept variant, two runs, identical sizes): score 0.3302 -> **0.2900** (-12.2%, 89% of variant a's gain).
  | corpus | E028 | variant a (all level 19) | E030 | E030 / E028 |
  |---|---:|---:|---:|---:|
  | md | 68,625 | 60,560 | 63,422 | 0.924 |
  | pdf | 20,547 | 20,334 | 20,379 | 0.992 |
  | office | 170,953 | 148,368 | 148,812 | 0.870 |
  | invoices | 4,941 | 3,343 | 3,370 | 0.682 |
  | synthetic-history | 16,926 | 15,073 | 15,655 | 0.925 |
  | repo-history | 1,108,165 | 891,517 | 907,851 | 0.819 |
  | repo-graph | 22,161 | 19,488 | 19,871 | 0.897 |
  | generated | 4,454 | 4,253 | 4,260 | 0.956 |
  - vs_dict: md 0.8925 -> 0.8248, pdf 0.8338 -> 0.8270, office 0.4370 -> 0.3804, invoices 0.3655 -> 0.2493, synthetic-history 0.2786 -> 0.2577, repo-history 0.8586 -> 0.7034, repo-graph 0.1954 -> 0.1752, generated 0.0254 -> 0.0243.
  - Time 36.7 s -> 51.6–52.5 s (1.4x): the kept plan is compressed at level 19 once per repack. Read cost (`read_whole`, `read_4k`): identical bytes decoded on every corpus.
  - Variant a is 1.7% smaller than E030, because there plans were also chosen and groups grown by level-19 sizes.
- **Caveat on the baseline:** the baseline is still zstd level 3 with a trained dictionary (the benchmark contract, unchanged here). Part of this gain is a stronger setting of the same codec, not a better structure. A level-19 baseline would be a benchmark change (its own entry, scores not comparable). The "vs zstd" ratios now compare a level-19 store with a level-3 baseline.
- Verdict: **kept.** The largest single gain since E022, from one tunable plus a final recompression pass. Next: other codecs as candidates (xz, brotli) chosen per piece, and domain transforms before the coder.

### E031 codec-candidates
- Branch / parent: exp/031-codec-candidates / E030
- Hypothesis: after E030, zstd frames are still most stored bytes. Offering two other codecs and keeping the smallest encoding per stored piece shrinks the store further, mostly on large frames, since neither codec uses the shared dictionary (source: measurement before E030, whole-file samples: xz -9e 0.83–0.85x and brotli-11 0.81–0.95x of zstd-19 on the repo's markdown, Rust source, server.log and metrics.csv)
- Change:
  - New `src/codec.rs`: "X" is an .lzma stream (LZMA at xz's preset 9e, dictionary sized to the input, so decoding a small frame allocates little); "R" is brotli at quality 11 (window sized to the input). Both decode as streams, so a group member's prefix read still stops at the member's end. Decoded bytes count in `decompressed_bytes`.
  - `src/dict.rs`: `encode_smallest` (zstd at the repo's level, .lzma or brotli, whichever is smallest); "X" and "R" are plain encodings for `is_plain`, `decode_plain` and `decode_prefix`.
  - `src/repack.rs` `recompress` uses `encode_smallest` for plain content (the dictionary stays "B"/"Z"). Plans are still compared with zstd level 3.
  - `Cargo.toml`: `xz2 =0.1.7` (static liblzma) and `brotli =9.0.0`, pinned like the other compressors.
  - Test `codecs_round_trip_and_read_prefixes`.
- Result (two runs, identical sizes): score 0.2900 -> **0.2859** (-1.4%).
  | corpus | E030 | E031 | ratio | frames now .lzma / brotli |
  |---|---:|---:|---:|---|
  | md | 63,422 | 60,560 | 0.955 | 0 / 3 |
  | pdf | 20,379 | 20,379 | 1.000 | – |
  | office | 148,812 | 148,607 | 0.999 | 2 / 1 |
  | invoices | 3,370 | 3,370 | 1.000 | – |
  | synthetic-history | 15,655 | 14,675 | 0.937 | 0 / 1 |
  | repo-history | 907,851 | 905,850 | 0.998 | 0 / 8 (77 frames stay zstd + dictionary, 697 KB) |
  | repo-graph | 19,871 | 19,871 | 1.000 | – |
  | generated | 4,260 | 4,260 | 1.000 | – |
  - vs_dict: md 0.8248 -> 0.7876, office 0.3804 -> 0.3798, synthetic-history 0.2577 -> 0.2416, repo-history 0.7034 -> 0.7018; the others unchanged.
  - Time 52.5 s -> 67.6 s (1.29x). The added repack time over all corpora (15.0 s) accounts for all of it. Read cost in decoded bytes is identical to E030 (as expected; the measure counts bytes, not CPU). xz and brotli decode more slowly per byte than zstd, which no figure here prices; read time is not measured separately.
  - **Attribution:** brotli wins 13 frames and .lzma 2, all large ones (groups or whole files). Small frames keep zstd with the dictionary. On repo-history the dictionary beats both new codecs on most frames.
- Verdict: **kept** (score improves; gain modest and concentrated in md and synthetic-history). Not tried: codecs with a shared dictionary (brotli's shared-dictionary format, LZMA2 preset dictionaries), which the repo-history result points to.

### E033 ppmd-codec
- Branch / parent: exp/033-ppmd-codec / E031
- Hypothesis: PPMd context modelling, offered as a fourth codec for the frames repack keeps, shrinks text and code leaves where zstd, .lzma and brotli leave most (source: E032 offline measurement on the stored leaves: PPMd order 8 at 0.929x on md, 0.966x on synthetic-history, 0.983x on repo-history)
- Change:
  - `src/codec.rs`: "P" mem:u8 + PPMd variant H (order 8) via `ppmd-rust =1.5.0` (pure Rust, CC0/MIT-0), with an end marker. The model size is 16x the input, clamped to 1–16 MiB and recorded in the byte, because a decoder allocates the whole model even to read a prefix. Streaming decoder, so member prefix reads stop early.
  - `src/dict.rs`: "P" is plain (`is_plain`, `decode_plain`, `decode_prefix`); `encode_smallest` tries it after .lzma and brotli.
  - Round-trip and prefix test extended to PPMd.
- Result (two runs, identical sizes): score 0.2859 -> **0.2831** (-1.0%).
  - md 60,560 -> 57,657 (0.952; 3 PPMd frames), synthetic-history 14,675 -> 14,402 (0.981; 1), repo-history 905,850 -> 896,937 (0.990; 25 frames, 341 KB); pdf, office, invoices, repo-graph, generated unchanged.
  - vs_dict: md 0.7876 -> 0.7498, synthetic-history 0.2416 -> 0.2371, repo-history 0.7018 -> 0.6949.
  - Time 67.6 s -> 69.4–69.5 s (1.03x, repack). Read cost in decoded bytes identical to E031.
  - md gains less than offline (0.952 vs 0.929), probably from the 1–16 MiB model cap.
- **Read speed (measured offline, not in the score):** on repo-history's 89 dumped leaves (4.76 MB decoded), decoding runs at 833 MB/s for zstd-19, 364 MB/s for brotli-11, and **9.9 MB/s for PPMd order 8** (pyppmd, the same variant H). So PPMd frames read about **84x slower** than zstd: about 6.5 ms instead of 0.08 ms per 64 KiB group. The benchmark cannot see this: total time is dominated by repack, and read cost counts bytes, not CPU per byte.
- Verdict: **kept under the benchmark contract** (bytes-only score improves; total time 1.03x). It is the clearest case so far of the score ignoring read speed: 1% fewer bytes for frames that read 84x slower. If read cost enters the objective (pending since E029), decode work should be weighted per codec, and PPMd would likely only pay for cold data. Reverting is one commit.

### E034 read-cost-score (benchmark change)
- Branch / parent: exp/034-read-cost-score / E033
- Change to the measurement: the score counts read cost, so a store that is smaller but slower to read is no longer rewarded by construction (source: E029, where blocks halved read cost but were discarded on bytes alone; E033, where PPMd frames read about 84x slower for 1% fewer bytes).
  - Per corpus: `(repack + w x read_whole) / (zstd_dict + w x read_baseline)`, w = `DEFAULT_READ_WEIGHT` (0.001, the planner's own weight: 1 stored byte = 1,000 decoded zstd bytes). Score = geometric mean, as before. `bytes_score` (the old score) is still reported, with `vs_dict_read` and `read_baseline` per corpus.
  - Read cost is now weighted per codec: `decode_cost()` (renamed from `decompressed_bytes()`) counts decoded bytes x the codec's decode time relative to zstd-19. Weights are measured by the ignored test `codec::tests::codec_decode_speeds` (Rust, this machine, the repo's own docs and kernel sources in 64 KiB pieces, two runs): zstd 637–672 MB/s; xz 49–50 MB/s (12.9–13.5x); brotli 173–178 MB/s (3.6–3.9x); PPMd 8.7–9.2 MB/s (69–77x). Constants: `XZ_READ` 13, `BROTLI_READ` 4, `PPMD_READ` 70. Function work is unchanged (bytes produced + fuel / 1,000).
  - The baseline reads each distinct file by decoding its own zstd frame once (`read_baseline` = distinct file bytes). Graph corpora: no file read measured, 0 on both sides, so their ratio is the bytes ratio.
  - **Scores before and after E034 are not comparable.**
- Result, E033 kernel (two runs identical; a third with the added `read_baseline` field gives the same values): score **0.6560**; bytes_score 0.2831 (= E033).
  | corpus | bytes | read_whole | read_baseline | vs_dict | vs_dict_read |
  |---|---:|---:|---:|---:|---:|
  | md | 57,657 | 648,587,590 | 177,939 | 0.7498 | **9.1637** |
  | pdf | 20,379 | 1,259,030 | 80,204 | 0.8270 | 0.8752 |
  | office | 148,607 | 430,860,281 | 2,802,002 | 0.3798 | **1.4706** |
  | invoices | 3,370 | 825,330 | 85,680 | 0.2493 | 0.3084 |
  | synthetic-history | 14,402 | 63,954,557 | 650,028 | 0.2371 | **1.2763** |
  | repo-history | 896,937 | 823,393,873 | 5,316,822 | 0.6949 | **1.3274** |
  | repo-graph | 19,871 | 0 | 0 | 0.1752 | 0.1752 |
  | generated | 4,260 | 1,322,701 | 688,414 | 0.0243 | 0.0318 |
  - Read costs are large because a group member's read decodes its group from the start (up to 64 KiB), and the group frames on md, synthetic-history and repo-history are now PPMd (70x), on office .lzma and brotli (13x, 4x).
- Diagnostic (one run; `encode_smallest` temporarily reduced to zstd, i.e. the E030 codec set, reverted): score **0.3511**, bytes_score 0.2900. md 0.9431, office 0.7911, synthetic-history 0.2833, repo-history 0.7265 (the others as above).
- Sensitivity to w (computed from the same runs; E033 kernel / zstd-only kernel): 0.0001: 0.3424 / 0.2970; 0.0003: 0.4308 / 0.3101; 0.001: 0.6560 / 0.3511; 0.003: 1.1297 / 0.4479. The E031 and E033 codecs lose at every weight tested.
- Verdict: reference for later experiments. Under the read-aware score, E031's and E033's codec choices (picked by bytes alone in `recompress`) are a large regression. The next experiment is to make `recompress` choose encodings by the same objective.

### E035 read-priced-codecs
- Branch / parent: exp/035-read-priced-codecs / E034
- Hypothesis: `recompress` picks codecs by bytes alone, which E034 showed is a large regression once reads count. Picking each frame's encoding by the score's own objective (stored bytes + w x codec decode weight x bytes decoded to read everything once) recovers at least the zstd-only result and keeps a slow codec only where a frame is rarely read (source: E034: score 0.6560, zstd-only diagnostic 0.3511)
- Change:
  - `src/dict.rs`: `encode_cheapest(bytes, reads)` replaces `encode_smallest`; `cost(encoded, reads)` = bytes + `read_weight` x `codec::read_weight(tag)` x reads. Test `codec_choice_prices_reads`.
  - `src/codec.rs`: `read_weight(tag)`, the E034 weights per encoding.
  - `src/group.rs`: `member_end` (a member entry's group prefix and range end).
  - `src/repack.rs` `recompress`: a group's reads are the sum of its members' range ends (a member read decodes its group up to there); any other content is read whole. A stored encoding is replaced only if the cost drops.
- Variants:
  - a: each content counted as read once. Score **0.3552**. Worse than zstd-only only on synthetic-history (0.2833 -> 0.3114): its single leaf is the **template** all 21 revisions are filled from, read whole on every revision read. The model counted it once and chose brotli (read cost 1.74 M -> 4.44 M).
  - b (kept): each content (or group member) read once plus once per derivation argument naming it (a template per filled document, a slice source per sliced document). Score **0.3510**; two runs identical.
- Result (b): score 0.6560 -> **0.3510**; bytes_score 0.2831 -> 0.2900.
  - Per corpus vs_dict_read: md 9.1637 -> 0.9431, pdf 0.8752, office 1.4706 -> 0.7911, invoices 0.3084, synthetic-history 1.2763 -> 0.2833, repo-history 1.3274 -> 0.7263, repo-graph 0.1752, generated 0.0318. Bytes: md 57,657 -> 63,422, synthetic-history 14,402 -> 15,655, repo-history 896,937 -> 907,280 (read 823.4 M -> 33.9 M), office 148,607 -> 148,812 (read 430.9 M -> 162.9 M).
  - Codec choices: every frame zstd except one brotli frame (repo-history's `uv.lock`, 25,558 B, read once). .lzma and PPMd are never chosen.
  - Against the zstd-only diagnostic (0.3511): better by 0.0001, from that one frame. Time 70.8 s vs 57.3 s for zstd only (1.24x): trying .lzma, brotli and PPMd on every kept frame costs ~13.5 s of repack and buys almost nothing at w = 0.001.
- Verdict: **kept** (score 0.6560 -> 0.3510). E031 and E033 were wins only under the bytes-only score. Under the read-aware score, slow codecs pay for almost nothing in this benchmark, where groups are read member by member and templates once per document. Open: drop the .lzma and PPMd candidates (their decoders would stay, so stores written since E031 still read) to save the repack time, unless a lower read weight for cold data is wanted.

### E036 one-objective
- Branch / parent: exp/036-one-objective / E035
- Hypothesis: repack still chooses whole plans (dictionary x template induction) and groups by bytes alone, while the score and `recompress` now use bytes + w x read cost. Choosing plans and groups by the same objective lowers the score, mostly where group decoding dominates reads (md, office, repo-history) (source: E034/E035 read costs; E029's invoices anomaly, from a group step and a plan choice using different objectives)
- Change:
  - `src/repack.rs`:
    - `ReadModel` (moved out of `recompress`): a content is read once plus once per derivation argument naming it; a group is decoded up to each member's end per read of that member.
    - `Repo::plan_cost(files)` = content bytes + `read_weight` x (decoded bytes weighted per codec + output bytes of files rebuilt by functions).
    - repack picks the plan with the lowest `plan_cost` (was: fewest content bytes) and applies it only if its cost, after `recompress`, is below the current store's.
  - `src/group.rs` `plan_group`: a group must lower bytes + w x (sum of member ends) against its members stored alone (bytes + w x their own length); `saved` is f64, and `group_small` keeps the proposal saving the most cost.
- Result (three runs identical, the third after moving `ReadModel` above the tests for clippy): score 0.3510 -> **0.3450**; bytes_score 0.2900 -> 0.2931.
  | corpus | bytes E035 -> E036 | read_whole E035 -> E036 | vs_dict_read | group members |
  |---|---|---|---|---|
  | md | 63,422 -> 65,549 | 9.27 M -> 3.09 M | 0.9431 -> **0.8906** | 333 -> 123 |
  | pdf | 20,379 | 1.26 M | 0.8752 | 44 |
  | office | 148,812 -> 151,433 | 162.9 M -> 123.1 M | 0.7911 -> **0.6968** | 656 -> 571 |
  | invoices | 3,370 | 0.83 M | 0.3084 | 30 |
  | synthetic-history | 15,655 -> 16,083 | 1.74 M -> 2.06 M | 0.2833 -> 0.2954 (worse) | 3 -> 25 |
  | repo-history | 907,280 -> 909,859 | 33.9 M -> 30.2 M | 0.7263 -> 0.7253 | 743 -> 643 |
  | repo-graph | 19,871 -> 19,968 | (not measured) | 0.1752 -> 0.1760 (worse) | 50 -> 35 |
  | generated | 4,260 | 1.32 M | 0.0318 | 5 |
  - Time 70.8 s -> 74.1 s (1.05x; `plan_cost` decodes every stored frame of each candidate plan).
  - **Where the estimate and the measurement disagree:**
    - synthetic-history: the chosen plan groups 25 members instead of 3, and measured read cost rises 18%.
    - repo-graph: the planner now trades bytes for read cost there, but the score measures no read for graph corpora.
    - Both are limits of the estimate (members read through derivations, graph reads), not of the objective.
- Verdict: **kept** (score improves; md and office gain most). The planner, the group step, `recompress` and the score now use one objective. Next: seekable blocks inside groups (E029), re-tested under this objective.

### E037 seekable-groups-read-aware
- Branch / parent: exp/037-seekable-groups-read-aware / E036
- Hypothesis: E029's seekable blocks were discarded only because the score ignored reads, and because repack's plan choice and the group step used different objectives. Under the read-aware score (E034) and one objective (E036), storing a group as independently decodable blocks where it lowers bytes + w x read cost improves the score, most where group decoding dominates reads (source: E029: md, pdf and office read cost 0.48–0.73x for 2–3.5% more bytes; E036 read costs)
- Change (E029's mechanism, ported by hand onto the current kernel):
  - `src/dict.rs`: plain encoding "K" dict[4] block count { frame_len } frames (`block_frames`, zstd at `STORE_LEVEL` with the dictionary per block; `unblock`; `block_size`); `decode_blocks`; "K" is plain for `is_plain`, `dict_of` and `needs` (gc keeps its dictionary); `with_dict` generalises the dictionary retry.
  - `src/group.rs`: `member_span` (group prefix, start, end) replaces `member_end`; `read_member` decodes only the blocks of a "K" group the member overlaps (still verified by the member's id). Test `blocked_group_members_read_back` (E029's, ported).
  - `src/repack.rs`: `ReadModel` keeps each member's range and read count. `block_reads` = bytes of the blocks each member read overlaps. `recompress` also offers a group "K" with blocks of 4 KiB and 16 KiB when a dictionary exists, kept if it lowers bytes + w x reads. `plan_cost` prices "K" groups by `block_reads`.
- Result (two runs identical): score 0.3450 -> **0.3363** (-2.5%); bytes_score 0.2931 -> 0.3033.
  | corpus | bytes E036 -> E037 | read_whole E036 -> E037 | vs_dict_read | "K" groups |
  |---|---|---|---|---|
  | md | 65,549 -> 66,133 | 3.09 M -> 2.14 M | 0.8906 -> 0.8858 | 1 |
  | pdf | 20,379 -> 20,853 | 1.26 M -> 0.69 M | 0.8752 -> 0.8714 | 1 |
  | office | 151,433 -> 170,571 | 123.1 M -> 48.1 M | 0.6968 -> **0.5550** | 14 |
  | invoices | 3,370 -> 3,662 | 0.83 M -> 0.50 M | 0.3084 -> 0.3058 | 1 |
  | synthetic-history | 16,083 | 2.06 M | 0.2954 | – |
  | repo-history | 909,859 -> 910,740 | 30.2 M -> 29.5 M | 0.7253 -> 0.7255 | 2 |
  | repo-graph | 19,968 -> 20,800 | (not measured) | 0.1760 -> **0.1834** (worse) | 1 |
  | generated | 4,260 | 1.32 M | 0.0318 | – |
  - Time 74.1 s -> 82.3 s (1.11x; each group is compressed in blocks twice more).
  - **Attribution:**
    - The gain is mostly office: 14 blocked groups cut its read cost 61% for 13% more bytes.
    - repo-graph loses because the planner prices reads for graph content, which the score does not measure (E036's limit).
    - repo-history is flat: blocking helps two groups, but the extra bytes cancel the read saved.
- Verdict: **kept** (score improves). E029's hypothesis is confirmed once the objective counts reads and is the same everywhere. Remaining mismatch: graph corpora (the planner prices reads the benchmark does not measure); either measure graph reads in the benchmark or exclude graph content from read pricing.

### E038 archive-slow-codecs
- Branch / parent: exp/038-archive-slow-codecs / E037
- Hypothesis: .lzma and PPMd have not been chosen for any frame since codec choice became read-priced (E035–E037; only brotli was, for 1–5 frames). Archiving them (no longer tried, still decodable) leaves every size unchanged and saves repack time (source: by_kind in E035–E037: no "X" or "P" frame on any corpus)
- Change:
  - `src/codec.rs`: `CODECS`, a table of (tag, encoder, active). Archived (inactive): .lzma "X" and PPMd "P". Active: brotli "R". An archived codec keeps its encoder, decoder and dependency, so frames stored with it still read and flipping `active` brings it back (e.g. for cold data at a lower read weight).
  - `src/dict.rs`: `encode_cheapest` tries only active codecs. Test `every_codec_in_the_table_still_decodes` (archived codecs included).
  - Chosen over deleting the codecs (user preference: archive rather than drop).
- Result (two runs identical): score **0.3363 -> 0.3363**; every size, read cost and ratio byte-identical to E037. Time 82.3 s -> 78.0–78.7 s (-5%). The saving is smaller than E035's estimate (~13.5 s for three extra codecs), since most recompress time is now blocks, zstd-19 and brotli.
- Verdict: **kept** (score holds; less repack time; capability kept as archived).

### E039 graph-read-cost (benchmark change)
- Branch / parent: exp/039-graph-read-cost / E038
- Change to the measurement: graph reads count in the score.
  - Since E034 the score counted read cost for file corpora only (graph corpora: 0 on both sides). Since E036 the planner prices reads for graph content too, so on repo-graph it traded bytes for read speed the score could not see (repo-graph worse in E036 and E037).
  - Rather than drop graph content from read pricing, the benchmark now measures graph reads: IKAM's graph is its core read path, and the planner should not learn to ignore it (source: E036/E037 repo-graph regressions).
  - `examples/bench.rs` `graph_read_cost`: after the exactness check (which warms the dictionary), every version's graph is read back once via `graph_nodes`, counting `decode_cost` only around those calls (finding the graph ids from commits and trees is not counted, as file reads count content only). The baseline is decoding each version's edge list once (`read_baseline` = distinct snapshot bytes), as for files.
  - Only repo-graph's ratio changes. **Scores before and after E039 are not comparable.**
- Result, E038 kernel (four runs identical: two counting commit and tree decoding too, two counting graph reads only, same figures): score **0.3588** (E038 under the old measurement: 0.3363); bytes_score 0.3033.
  - repo-graph: read_whole 0 -> **14,422,902** against read_baseline 1,006,283 (14.3x the baseline); vs_dict_read 0.1834 -> **0.3078**. Every other corpus is unchanged.
- Verdict: reference. Reading every version's graph costs the kernel 14x what decoding the edge lists costs the baseline, so graph read cost is now visible as a target.

### E040 structured-corpus (benchmark change)
- Branch / parent: exp/040-structured-corpus / E039
- Change to the measurement: a ninth corpus, `structured`, of real structured data that no short program generates, so domain transforms (fields, columns) have something to act on (source: E032, which found no CSV or log content stored as a leaf: the generated corpus's CSV and log are stored as programs).
  - Built deterministically from this repository's history up to the commit `repo-graph` pins (0ca74e4, 38 commits), with a snapshot at every fourth commit and at the last (10 snapshots, 30 files, 8,885,967 B). Each snapshot has:
    - `inventory.csv`: a full-table export, every file's path, mode, size and blob id (~2,600 rows, ~330 KB);
    - `changes.ndjson`: an append-only log, one JSON object per file change so far (commit, ISO date, author name, path, lines added and deleted; up to 2,906 rows);
    - `summary.json`: files and bytes per top-level directory.
  - Author emails are left out. All files parse as CSV, NDJSON and JSON (checked once on a temporary dump).
  - The other eight corpora are unchanged. **Scores before and after E040 are not comparable.**
- Result, E039 kernel (two runs identical): score **0.3205** over 9 corpora; bytes_score 0.2712. The other eight corpora are byte-identical to E039.
  - structured: 8,885,967 B in; kernel 110,873 B against zstd + dictionary 1,001,028 B (vs_dict **0.1108**; zstd per file 1,093,528). Read cost 20.5 M against a baseline 8.9 M (vs_dict_read **0.1301**). Time: repack 7.0 s; total 77.7 s -> 90.3 s.
  - Stored as: 21 derivation records ("D" 2,835 B, "d" 2,187 B), 29 group members, and leaves of 3 "Y" frames (86,075 B) plus the dictionary ("Z" 17,844 B). The kernel dedups across versions (repeated inventory rows, the append-only log), which per-file compression cannot. Most of what is left is the unique new rows, largely 40-hex-digit blob ids.
- Verdict: reference. The first corpus where field- and column-level transforms have a measurable target (the 86 KB of "Y" leaves).

### E042 graph-reads-distinct (benchmark correction)
- Branch / parent: exp/042-graph-reads / E041 (merged state: E040)
- Change to the measurement, a correction to E039:
  - E039 read the graph of every commit (38) but counted the baseline as each **distinct** edge list once (`distinct_bytes`), the rule for file corpora. Only 13 of the 38 commits have distinct graphs, so the kernel's graph read cost was overcounted about 2.9x against a baseline counted once per distinct version.
  - `graph_read_cost` now reads each distinct graph id once, matching both the baseline and file corpora.
  - **Scores before and after E042 are not comparable**; E039's and E040's reported figures for repo-graph's read cost (14,422,902) are superseded.
- Result, E040 kernel (two runs identical): score **0.3097** (E040 as reported: 0.3205); bytes_score 0.2712.
  - repo-graph: read_whole 14,422,902 -> **5,024,570** against read_baseline 1,006,283 (5.0x, not 14.3x); vs_dict_read 0.3078 -> **0.2256**. Every other corpus is unchanged.
- Verdict: reference (measurement fixed). Graph reads still cost 5x the baseline's.

### E043 graph-single-pass
- Branch / parent: exp/043-graph-single-pass / E042
- Hypothesis: `graph_nodes` found a graph's leaf chunks with `graph_leaves` (which decodes every chunk to learn its kind) and then decoded each leaf again for its nodes, so every leaf was read twice. One traversal that collects nodes as it goes halves leaf decoding, with storage unchanged (source: reading `src/graph.rs` while investigating E042's 5x graph read cost)
- Change: `src/graph.rs`: `graph_nodes` uses a recursive `collect_nodes`, reading each chunk once. `graph_leaves` stays for graph diffs, which need leaf ids.
- Result (two runs identical): score 0.3097 -> **0.3062**; bytes_score 0.2712 (unchanged; every size byte-identical).
  - repo-graph: read_whole 5,024,570 -> **2,518,350** (2.5x the baseline's 1,006,283); vs_dict_read 0.2256 -> **0.2037**. Every other corpus is unchanged.
- Verdict: **kept** (score improves; a read-path fix, no storage change). Graph reads still cost 2.5x the baseline. Remaining hypothesis: the planner's read model counts a graph chunk's content as read once, though each chunk is read once per distinct graph version that contains it, so it under-prices graph reads when grouping or blocking.

### E045 compact-graph-chunks
- Branch / parent: exp/045-compact-graph-chunks / E043
- Hypothesis: graph reads cost 2.5x the baseline because the canonical chunk encoding is large: every count and string length is a 4-byte integer, every node label (sorted paths with long shared prefixes) is written in full, and every edge repeats its label (`imports`, `links`, `uses`). Front-coding labels against their neighbour, front-coding edge labels against the previous edge, and using varints shrinks what each graph read decodes, and the stored chunks (source: E044's finding that the remaining graph read cost is the decoded chunk size; front coding, proposed as "front-coded graph labels" in the next-experiments list given with E025, and the sorted-dictionary encodings of HDT reviewed in `docs/research/graph-storage.md`)
- Change:
  - `src/object.rs`: graph chunks set `COMPACT` (0x80) in their level byte. Counts and lengths are varints. Node labels and child first labels are front-coded against the previous one in the chunk (`put_front`: shared prefix length on a char boundary, then the rest). Edge labels are front-coded against the previous edge label in the chunk. Edge keys stay 8 bytes (edges name targets by key, so cycles are allowed).
  - Decoding accepts both layouts, so stores written before E045 still read. A graph written now gets a different id than before E045 (an object's id is the hash of its canonical encoding).
  - Test `graph_chunks_decode_in_both_layouts`: a pre-E045 chunk decodes to the same graph; the compact form round-trips and is smaller.
- Result (two runs identical): score 0.3062 -> **0.3005**; bytes_score 0.2712 -> 0.2688.
  - repo-graph: bytes 20,800 -> 19,293; read cost 2,518,350 -> **471,248** (0.47x the baseline's 1,006,283, from 2.5x); vs_dict 0.1834 -> 0.1701; vs_dict_read 0.2037 -> **0.1727**.
  - generated: 4,260 -> 4,245 B (its function-library graph also uses graph chunks); vs_dict_read 0.0318 -> 0.0317.
  - The other corpora are byte-identical. Time 79.9 s -> 87.0 s (not attributed; within run-to-run variation seen before, sizes unaffected).
- Verdict: **kept**. Graph reads now cost less than decoding the baseline's edge lists. Object-format note: graph ids change for graphs written from now on; old graphs keep reading.

### E048 read-model-distinct-sources
- Branch / parent: exp/048-lazy-fill / E045 (the E046 and E047 entries live on their branches)
- Hypothesis, as first stated (C1 in `docs/research/latent-reasoning.md`, from latent-CoT "trace shortening"): md reads cost 12x their baseline because layered templates rebuild intermediate templates whole. Lazy `fill` would remove that work.
- What instrumentation showed: the premise was wrong. Read work per function kind during the scored whole-file reads, plus the planner's predicted read cost (`plan_cost` minus bytes, over `read_weight`), against the measured cost:
  - md, pdf and invoices are all decoding with no function work (md 2,135,673). The planner predicts this within 1% (2,155,935): grouped members decode their group up to their own end.
  - Layered `fill` work exists only in synthetic-history (619,972), office (40,394) and structured (23,093).
  - The planner **over-predicts** structured by 58% (32.4 M predicted vs 20.5 M measured) and repo-history by 40% (41.1 M vs 29.5 M). Cause: `read_model` charges a slice source once per derivation *argument*, but a read caches what it decodes, so a concat with many ranges of one source decodes it once.
  - It under-predicts synthetic-history (1.17 M vs 2.06 M, the unmodelled inner `fill` work) and generated (0.73 M vs 1.32 M, WASM fuel).
- Hypothesis, as tested: count a source once per derivation naming it. The planner then stops over-pricing slice plans and chooses cheaper ones.
- Change: `src/repack.rs`, `read_model` counts distinct argument ids per derivation. Test `read_model_counts_a_source_once_per_derivation` fails without the fix (3 vs 2).
- Result (two runs identical; tests, clippy and fmt clean; every corpus exact with a clean fsck): score 0.3005 -> **0.2972**; bytes_score 0.2688 -> 0.2672.
  - structured: bytes 110,873 -> 107,607; read 20,525,163 -> 17,882,915; vs_dict_read 0.1301 -> **0.1243**. Prediction now 17.94 M vs 17.88 M measured.
  - synthetic-history: bytes 16,083 -> 15,671; read 2,055,193 -> 1,494,074 (the layered fills are no longer chosen); vs_dict_read 0.2954 -> **0.2796**.
  - repo-history: bytes 910,740 -> 910,278; read 29,451,124 -> 29,724,965; vs_dict_read 0.7255 -> 0.7253. Still over-predicted (35.2 M vs 29.7 M), cause not yet found.
  - Other corpora are byte-identical. Time 87.0 s -> 77.5 s.
- Verdict: **kept**. Remaining model gaps: inner `fill` and WASM fuel work are unmodelled, and repo-history is still over-predicted. md's 12x read cost is the priced cost of grouping, not template work.

### E051 proposer-swap-test
- Branch / parent: exp/051-proposer-swap-test / E048
- Hypothesis (C8 in `docs/research/latent-reasoning.md`, after "Do Latent Tokens Think?" arXiv:2512.21711). A proposer only carries instance-specific information if swapping or removing its output changes the result. Applied to the committed embedding order (`semantic/order.txt`, 1,498 ids, E019), which repack offers as one grouping proposal among five.
- Measurement: the bench was rerun with the proposal removed, shuffled (fixed seed), and reversed. These were temporary bench variants and were not kept. A diagnostic in `group_small` printed, per grouping pass, how many items the order covers and what each of the five proposals would save.
- Result: **every corpus is byte-identical in all three variants** (score 0.2972, the same read costs, all exact).
  - The order covers md (333 of 333 items per pass) and repo-history (62–658 of about 700–800 per pass).
  - In md it never wins a pass.
  - In repo-history it wins some passes, by up to 6% (75,642 vs 71,738). Those passes belong to candidate re-plans that lose on `plan_cost`, so the final store never contains its groups.
- Verdict: **refuted for every current corpus.** The embedding proposer has no effect on what is stored.
  - Kept: the hook (`Repo::proposed_order`), and the order file it reads.
  - Added: the swap test as a standard step for proposer experiments (`docs/experiments/README.md`, step 5).
  - Ties to E017–E020: measured cost already beats every resemblance signal tried.

### E055 transcript-corpus (benchmark change)
- Branch / parent: exp/055-transcript-corpus / E051
- Hypothesis (C7 in `docs/research/latent-reasoning.md`; the latent-CoT papers compress reasoning traces lossily): agent transcripts quote files, tool output and earlier turns verbatim. A store holding both the workspace and the transcript should dedup those quotes exactly, as slices of stored content.
- Benchmark change: new corpus `transcripts` (10th) in `examples/bench.rs`, deterministic and with no real session data.
  - Contents: the kernel's 20 source files pinned at 0ca74e4 (161,549 B), plus `session.jsonl`, a 300-step agent log.
  - Session steps: whole-file Reads (30%), 40-line range Reads (50%) and Greps (20%), each over a random source file.
  - Format, as real coding-agent logs are written: results quoted line-numbered (`%6d\t`) and JSON-escaped; every event carries a random uuid and its parent's.
  - Snapshotted every 75 steps (4 versions; the session grows to 1,269,866 B).
  - `json_str` now writes `\n` and `\t` as short escapes, as JSON encoders do. structured, the only other user of `json_str`, is byte-identical.
- Result (two runs identical; clippy clean; every corpus exact with a clean fsck):
  - transcripts: kernel 113,507 B vs zstd + dictionary 322,688 B. **vs_dict 0.3518, vs_dict_read 0.3715** (read 7,610,892 vs a 3,315,758 baseline).
  - Score over 10 corpora: **0.3039** (bytes_score 0.2746). Not comparable with 0.2972 over 9.
- Where the bytes go (diagnostic, not kept):
  - The planner stores the *latest* session as one compressed blob, and earlier sessions as prefix slices of it. The kernel's lead over the baseline is this version dedup.
  - No quote is stored as a slice of a source file. Line-number prefixes and JSON escaping break every byte run longer than one source line.
  - Bound, computed on the latest session with zstd 19 (long window) in Python:

    | how the latest session is compressed | bytes |
    |---|---|
    | alone | 75,821 |
    | with the raw sources as context | 50,659 |
    | with the sources line-numbered and escaped as quotes are (an oracle for a quote-aware function) | **24,250** |
    | the sources themselves | 40,206 |

  - A `quote` derivation (line-number and escape a slice of stored content) would let quotes be stored as functions of the files, saving up to about 51 K of the session's 76 K. That is about 45% of the corpus.
  - Separately, compressing a content with *related stored content* as its zstd context, rather than the one trained dictionary, saves 33% even without the transform.
- Verdict: **benchmark change kept.** It adds the workload C7 described, and shows exact dedup is blocked by quoting syntax, not by missing reuse.
- Next hypotheses, both priced by read cost:
  - a `quote` builtin or WASM function for line-numbered, escaped slices;
  - related-content zstd context (delta against stored content).

### E056 quote-function
- Branch / parent: exp/056-quote-function / E055
- Hypothesis (E055's bound; C7 in `docs/research/latent-reasoning.md`): a transcript's quotes are its sources' lines, numbered and JSON-escaped. A function that rebuilds that formatting from the stored sources turns about 92% of the session into a short op stream. E055's zstd bound with the quoted sources as context was 24.3 K for the session, against 75.8 K alone.
- Change:
  - `functions/quote/` (no_std Rust, built to `functions/quote.wasm`, 5,283 B) implements `quote(ops, names, sources...)`. Its ops are: a literal run; lines `a..a+n` of source `s` as `{n:>6}\t{line}` (Read style); or as `{name}:{n}:{line}` (grep style). Every quoted line is JSON-escaped and ends in `\n`.
  - `examples/bench.rs`: proposer `propose_quotes`. For a file of at least 64 KiB in the last snapshot, it greedily encodes the longest runs of quoted lines of the other files. It proposes the derivation through `put_derivation` (verified) only if at least half of the file is quoted.
  - **Kernel fix**, `src/repack.rs`: `replan` re-ingested files largest first. So a fact's output (the 1.27 MB session) was planned before its inputs (op stream, sources) existed, and the fact could not be verified as a candidate. The output was then stored as slice plans with literal blobs, and the fact was kept *as well*. `inputs_first` now orders each fact's inputs before its output (depth-first over hint arguments). Test `replan_puts_fact_inputs_first`.
- Steps measured:
  - Proposer alone (fact recorded, but repack plans the session first): transcripts 113,507 -> 132,781 B (worse). The session's slice plan kept 74.6 K of literal blobs next to the fact's inputs.
  - Also proposing prefix slices of the latest session for earlier versions changed nothing; repack re-derives plans.
  - With `inputs_first`: **71,507 B**.
  - Proposing quotes for every version instead of only the latest: 75,336 B, worse (four op streams and records). Latest only is kept.
- Result (two runs identical; tests, clippy and fmt clean; every corpus exact with a clean fsck): score 0.3039 -> **0.2929**; bytes_score 0.2746 -> 0.2622.
  - transcripts: 113,507 -> 71,507 B; read 7,610,892 -> 12,423,767 (the session is now rebuilt by WASM, priced); vs_dict 0.3518 -> **0.2216**; vs_dict_read 0.3715 -> **0.2575**.
  - Every other corpus is byte-identical, including office and generated, whose facts now also have their inputs planned first.
- Verdict: **kept.**
  - The ordering fix is general: any proposer's fact (a program, a container, a quote) can now replace its output's plan during repack.
  - The quote function is a proposed reconstruction function, not a kernel builtin; the kernel only verifies it.
- Remaining:
  - The session costs about 12.4 M of read work per read pass. The planner still doesn't model WASM work (E048).
  - Earlier session versions are slices of the WASM-built latest.

### E057 related-context
- Branch / parent: exp/057-related-context / E056
- Hypothesis (E055: the session compressed 33% smaller with the related sources as zstd context). Compressing a stored content with the most similar *other stored content* as its zstd context (a raw-content dictionary) captures similarity too fine for slices: matches shorter than the matcher's seeds, and shifted or edited text. The one trained dictionary is shared by everything and is small.
- Offline estimate before building (diagnostic, not kept): for every leaf of at least 2 KiB, zstd 19 with its most similar leaf (by shared shingles) as context, against its current stored size.
  - Savings: md 4.9 K, structured 9.7 K, repo-history 8.0 K, generated 1.4 K, repo-graph 1.2 K, transcripts 3.2 K.
  - This ignored how often a read decodes the context.
- Change:
  - `src/dict.rs`: new plain encoding `"Q" base[32] zstd_with(base's bytes)`. Decoding reads `base` from its stored encoding, which must not be `Q` or `S`, so decoding is never more than one step and never cycles; the base is verified against its id. `needs` names the base, so gc keeps it.
  - `src/repack.rs`, `recompress`:
    - candidate bases are the contents stored as B/Z/Y (not the dictionary) of at least 2 KiB, indexed by content-defined 16-byte shingles;
    - each content may take its most related base;
    - cost = stored bytes + `read_weight` x (own decoding + the base's length per decode event);
    - kept only if cheaper;
    - a content used as a base never becomes `Q`, and a `Q` is never a base.
  - `ReadModel::events` counts decode events; for a group that is one per member read. `plan_cost` prices a `Q`'s base decoding.
  - `src/ingest.rs`: ingest never drops the stored copy of a content that a `Q` uses as context (`is_related_base`).
  - Test `related_context_decodes_one_step_only`: a `Q` decodes, gc's `needs` names its base, and a `Q` whose base is a `Q` is refused.
- Result (two runs identical; tests, clippy and fmt clean; every corpus exact with a clean fsck): score 0.2929 -> **0.2920**; bytes_score 0.2622 -> 0.2604.

  | corpus | Q encodings | bytes | read | vs_dict_read |
  |---|---|---|---|---|
  | structured | 1 (17,844 B) | 107,607 -> 101,106 | 17.9 M -> 20.9 M | 0.1243 -> **0.1209** |
  | repo-history | 14 | 910,278 -> 908,059 | 29.7 M -> 30.7 M | 0.7253 -> 0.7243 |
  | repo-graph | 6 | 19,293 -> 19,213 | | 0.1727 -> 0.1721 |
  | office | 2 | 170,571 -> 170,492 | | 0.5550 -> 0.5549 |

  - md, pdf, invoices, synthetic-history, generated and transcripts are byte-identical. Their candidates were rejected by read pricing: md's two group frames would each decode the other, about 65 K, once per member read (127 reads).
  - Time 89 s -> 93 s.
- Verdict: **kept** (a small gain).
  - The offline estimate overstated the gain because it priced one read per leaf; the read-priced planner accepts about a quarter of it.
  - A new storage invariant (a `Q` base must stay stored, as a dictionary must) is enforced at the one non-gc deletion site and tested.

### E058 search-index (capability; reported, not scored)
- Branch / parent: exp/058-search-index / E057
- Hypothesis (E050: a coding agent's access to large files was dominated by grep scans, which decode everything): a trigram index (as in Google Code Search) lets a search decode only candidate contents. Its stored bytes pay for themselves once a workload runs enough searches per read pass.
- Change:
  - `src/search.rs`, three functions:
    - `Repo::build_search_index(files)` stores a trigram index as ordinary content: file ids, then per trigram a delta-varint posting list. The ref `meta/search-index` names it.
    - `Repo::search_index()` loads it.
    - `Repo::search(pattern, files, index)` returns the files containing `pattern`.
  - Search is exact whatever the index says: every candidate is decoded and scanned. Contents the index doesn't cover are always scanned. Patterns under 3 bytes scan everything.
  - Test `indexed_search_matches_a_full_scan`.
  - `examples/bench.rs` (reported, not scored): 200 deterministic grep-like queries per file corpus (an identifier from a random line, as in E050), measured after everything scored.
  - It reports the decode cost per query with and without the index, the index's stored bytes, and whether both return the same files. The index's own decoding is counted once and amortised over the queries.
- Result (two runs identical; tests, clippy and fmt clean): score **0.2920 unchanged**; every corpus byte-identical; indexed and full-scan results identical everywhere.

  | corpus | decoded per query: scan -> indexed | index bytes | pays above (searches per read pass) |
  |---|---|---|---|
  | generated | 1.32 M -> 0.57 M (0.43) | 13,938 | 19 |
  | repo-history | 30.7 M -> 9.3 M (0.30) | 731,983 | 34 |
  | office | 48.1 M -> 16.4 M (0.34) | 1,201,155 | 38 |
  | md | 2.14 M -> 0.41 M (0.19) | 111,826 | 65 |
  | transcripts | 12.4 M -> 12.0 M (0.97) | 72,343 | 178 |
  | pdf | 0.69 M -> 0.34 M (0.49) | 78,588 | 224 |
  | structured | 20.95 M -> 20.90 M (1.00) | 27,927 | 575 |
  | invoices, synthetic-history | no reduction (every file holds the queried words) | | never |

  "Pays above" = index bytes / (0.001 x decode saved per query): the searches per full read pass at which the index costs less than the scans it avoids.
- Verdict: **kept as a capability**; the score holds.
  - E050's agent trace ran roughly 100 searches per read pass of the files it touched. At that rate the index would pay on generated, repo-history, office and md, but not on pdf, transcripts, structured, invoices or synthetic-history. There, words recur in every file, or the files that hold them are the big ones.
  - Whether search belongs in the score is a decision about the workload, not something the benchmark can settle. The numbers above give its size.
  - The index is large: full 32-byte ids, and a posting for every distinct trigram. It is bigger than the md store itself. Abbreviated ids, or indexing only rarer trigrams, are the obvious next levers if it is ever scored.

### E059 measured-plan-cost
- Branch / parent: exp/059-measured-function-work / E058
- Hypothesis (open since E048): the planner's read estimate is wrong where the score's read term is not a sum of whole decodes.
  - Function work: E048 found synthetic-history's nested `fill` levels and generated's WASM fuel unmodelled.
  - Read counts: repo-history was still over-predicted by 18%. Cause, found here: `read_model` counted every content as read once for itself (`1 + uses`), including intermediates (slice literals, templates) that are only ever read through the files that use them. It counted each use once, whatever its reader's own read count. It also counted the arguments of derivations that a read never evaluates: those of contents stored plainly, and of alternatives after the first.
  - Plans compared on wrong costs can be chosen wrongly.
- Change, all in `src/repack.rs`:
  - `plan_cost(roots)` now **measures** the score's read term instead of modelling it: it reads every root file once and counts bytes decoded (codec-weighted) plus function work, as the benchmark does. Plan choice and repack's acceptance test use it.
  - `read_model(roots)`, which `recompress` still uses per content, counts reads recursively from the roots. A root is read once. Any content is read once per read of each content whose read evaluates a derivation naming it: none for content stored plainly, otherwise the first derivation. Each source is still counted once per derivation (E048).
  - Removed `dict::block_size`, no longer used.
- Prediction check (diagnostic, not kept), measured read against the planner's estimate with only the function work measured:
  - generated 1.32 M against 1.33 M (was 0.73 M);
  - synthetic-history 1.49 M against 1.52 M (was 1.17 M).
  - With the measured `plan_cost`, prediction equals measurement by construction (up to read_range's walk vs a rebuild).
- Result (two runs identical; tests, clippy and fmt clean; every corpus exact with a clean fsck; search results identical): score 0.2920 -> **0.2918**; bytes_score 0.2604 -> 0.2600.
  - office: 170,492 -> 165,802 B; read 48.1 M -> 51.9 M; vs_dict_read 0.5549 -> **0.5524**.
  - transcripts: 71,507 -> 72,592 B; read 12.4 M -> 11.2 M; vs_dict_read 0.2575 -> **0.2569**. More bytes, less read: the measured cost prefers it.
  - repo-history: 908,059 -> 907,329 B; vs_dict_read 0.7243, unchanged at 4 decimals.
  - Other corpora are byte-identical. Time 93 s -> 106 s (each candidate plan is now read once to price it).
- Verdict: **kept.** Plans are compared on the measured objective, and the per-content model no longer over-counts intermediates.

### E060 repack-acceptance
- Branch / parent: exp/060-repack-acceptance / E059
- Hypothesis (open since E052): repack accepted a re-plan by content and derivation bytes plus read cost, but the score counts the whole store. The objects that outlive a re-plan (trees, commits, graph chunks) are re-encoded under the new plan's dictionary, so a re-plan that shrinks content could grow the store: repo-graph grew 19,293 -> 19,450 on a second repack in E052. Office reported `applied` on every repack while its store stayed the same.
- Reproduced on E059 with three repacks per corpus (temporary bench change):
  - office is now idempotent; E059's measured `plan_cost` fixed its repeated no-op apply.
  - repo-graph still applies a second re-plan whose content shrinks (8,660 -> 6,131) while the store grows (19,213 -> 19,450), then re-applies it on the third.
  - generated (-32 B) and transcripts (-35 B) shrink on the second repack without re-planning, through the tree unsharing and gc that start every repack.
- Change, all in `src/repack.rs`:
  - The acceptance test adds, on both sides, the stored size of every lasting non-derivation object: its current size now, and its size encoded under the fresh plan's dictionary.
  - `repack` ends with `unshare_trees` and `gc`. Trees whose content the new plan no longer shares go back to object form within the same repack, instead of at the next one.
  - With both, a second and a third repack change nothing on any corpus (`applied false`, every size equal).
- Result (two runs identical; tests, clippy and fmt clean; every corpus exact with a clean fsck; search results identical): score 0.2918 -> **0.2903**; bytes_score 0.2600 -> 0.2586.
  - repo-graph: 19,213 -> 18,348 B; vs_dict_read 0.1721 -> **0.1644**.
  - generated: 4,245 -> 4,213 B.
  - transcripts: 72,592 -> 72,557 B.
  - Other corpora are byte-identical.
- Verdict: **kept.** Repack is now a fixed point after one call, and its acceptance test is the score's objective. That is a precondition for any background or anytime optimisation loop (E052).

### E062 search-in-score (benchmark change)
- Branch / parent: exp/062-search-in-score / E060
- Decision (the user's): include search in the score. E058 measured search as reported, not scored. E050's agent trace ran about 100 grep scans per read pass over the files it touched, so `SEARCH_RATE = 100` searches per read pass. That is a workload assumption, and it is one named constant.
- Benchmark change (`examples/bench.rs`, `docs/experiments/README.md`):
  - Per file corpus the score is now `(repack + w x (read_whole + q x search)) / (zstd_dict + w x (1 + q) x read_baseline)`. The baseline searches by decoding every distinct file.
  - `search` is the kernel's decode cost per query through `Repo::search`: 100 deterministic identifier queries, as in E058, using any index the store holds. Without an index every query decodes every file, so one query is measured.
  - Every result must equal a scan of the original bytes (a new hard gate).
  - Graph corpora have no search term.
  - New fields: `vs_dict_search` and `search`; `_summary.read_score` keeps the E034–E061 score.
  - E058's reported-only search fields are removed. The kernel API (`Repo::search`, `build_search_index`) stays. This change re-measures the current kernel, which stores no index.
- Result (two runs identical; every corpus exact with a clean fsck; every search result exact): **score 1.5736**; read_score 0.2903 (unchanged); bytes_score 0.2586.

  | corpus | vs_dict_read (no search) | vs_dict_search | search per query (= a full read pass) | baseline per query |
  |---|---|---|---|---|
  | md | 0.8858 | 2.971 | 2,135,673 | 177,939 |
  | pdf | 0.8714 | 2.765 | 689,862 | 80,204 |
  | office | 0.5524 | **8.018** | 51,883,141 | 2,802,002 |
  | invoices | 0.3058 | 2.438 | 498,922 | 85,680 |
  | synthetic-history | 0.2796 | 1.318 | 1,494,613 | 650,028 |
  | repo-history | 0.7243 | 2.227 | 31,308,797 | 5,316,822 |
  | repo-graph | 0.1644 | 0.1644 | (graph) | |
  | generated | 0.0315 | 0.564 | 1,322,701 | 688,414 |
  | structured | 0.1209 | 1.168 | 20,950,811 | 8,885,967 |
  | transcripts | 0.2568 | 1.826 | 11,169,671 | 3,315,758 |

- What this shows: with 100 searches per pass the score is dominated by read cost, and the kernel's per-file read cost is 2–18x the baseline's. Sources:
  - group prefix decoding (md, pdf, invoices);
  - container rebuilds (office: deflate-pack, 18x);
  - slice sources and functions (repo-history, structured, transcripts).

  The kernel loses to the baseline on 8 of 10 corpora (above 1.0); it still wins on repo-graph and generated.
- Verdict: **benchmark change kept** (the user's decision). Scores are not comparable with E034–E061.
- Next levers, under the new objective:
  - The kernel stores and uses the trigram index when it pays. Per E058 it cuts search decode to 0.19–0.43x on md, office, repo-history and generated.
  - The planner must weigh the search workload: today it optimises bytes plus one read pass, so it keeps plans that are cheap to store but costly to scan.

### E063 operation-profile (benchmark change, reported, not scored)
- Branch / parent: exp/063-operation-profile / E062
- Why (the user's direction): the score must account for the full set of operations and for real time and space, not proxies. Until now it counted stored bytes plus two proxies: decoded bytes and function work. Those proxies leave out ingest, repack and verification, and they never measured time or memory.
- Benchmark change (`examples/bench.rs`), reported per corpus under `ops`, not scored:
  - A counting global allocator measures each operation's peak heap above its start.
  - Every operation's wall time is measured (median of 3 for reads; one run for ingest, repack and fsck), for the kernel and for the baseline. The baseline is the corpus as `baselines` stores it: zstd 3 with a trained dictionary.
  - Operations:
    - ingest (the baseline's is dictionary training plus compression);
    - repack (kernel only);
    - read every distinct file;
    - a 4 KiB range read of each file (the baseline decompresses, then slices);
    - one search (the baseline decompresses all files and scans);
    - fsck (the baseline decompresses all files and hashes them with BLAKE3);
    - for repo-graph, reading every distinct graph.
  - Times vary between runs: about ±15% between two runs. Every scored field was identical across the two runs.
- Result, median ms, kernel vs baseline (two runs; every corpus exact with a clean fsck; score 1.5736 unchanged):

  | corpus | ingest | repack | read all | range all | search | fsck | kernel peak (read) | baseline peak (read) |
  |---|---|---|---|---|---|---|---|---|
  | md | 96 vs 36 | 5,573 | 18.7 vs 0.41 (45x) | 20.6 vs 0.45 | 18.6 vs 0.96 | 20.5 vs 0.74 | 88 KB | 186 KB |
  | pdf | 25 vs 14 | 814 | 1.8 vs 0.11 (16x) | 1.6 vs 0.12 | 1.9 vs 0.34 | 2.7 vs 0.19 | 79 KB | 81 KB |
  | office | 1,541 vs 142 | 38,836 | 458 vs 0.81 (**569x**) | 461 vs 0.81 | 474 vs 9.9 | 542 vs 1.8 | 2.3 MB | 2.8 MB |
  | invoices | 36 vs 8.5 | 721 | 1.0 vs 0.04 (27x) | 1.1 vs 0.04 | 1.0 vs 0.03 | 1.3 vs 0.09 | 68 KB | 86 KB |
  | synthetic-history | 26 vs 39 | 564 | 2.5 vs 0.07 (36x) | 1.8 vs 0.08 | 2.7 vs 0.59 | 3.2 vs 0.26 | 126 KB | 651 KB |
  | repo-history | 711 vs 425 | 43,408 | 103 vs 8.7 (12x) | 98 vs 8.9 | 110 vs 14.3 | 141 vs 10.3 | 375 KB | 5.3 MB |
  | repo-graph | 92 vs 52 | 720 | 14.9 vs 0.64 (23x) | | | 4.9 vs 0.61 | 192 KB | 1.0 MB |
  | generated | 13 vs 6.8 | 6,405 | 366 vs 1.2 (**308x**) | 388 vs 0.82 | 392 vs 3.0 | 409 vs 1.2 | **67 MB** | 0.7 MB |
  | structured | 267 vs 336 | 7,675 | 20.7 vs 5.1 (4x) | 14.3 vs 4.3 | 20.5 vs 4.7 | 19.6 vs 6.2 | 2.2 MB | 8.9 MB |
  | transcripts | 77 vs 119 | 4,239 | 347 vs 1.5 (**233x**) | 352 vs 1.3 | 359 vs 1.5 | 387 vs 1.9 | 11 MB | 3.3 MB |

- What the real numbers show:
  - **The decode proxy badly understates real read time.** Read cost ratios, kernel over baseline:

    | corpus | proxy | real |
    |---|---|---|
    | office | 18x | 569x |
    | generated | 1.9x | 308x |
    | transcripts | 3.4x | 233x |
    | md | 12x | 45x |

    WASM interpretation is the main reason (generated, transcripts' quote function). Its fuel/1000 pricing is two orders of magnitude too low. Rebuilding zips (deflate re-compression) explains office. Fixed per-read overheads (ledger lookups, verification hashing, groups) explain the rest.
  - Range reads are no faster than whole reads in the kernel, because leaves decode whole (E046/E047).
  - The kernel uses less read memory than the baseline on most corpora (it streams group prefixes and keeps no decoded corpus). The exceptions are generated (67 MB, WASM program memory) and transcripts (11 MB).
  - Ingest and repack are slower everywhere. Repack takes up to 43 s, against a baseline with no maintenance step.
- Verdict: **benchmark change kept** (measurement only). How these costs combine into the score is still to decide.

### E064 cost-score (benchmark change)
- Branch / parent: exp/064-cost-score / E063
- Decision (the user's direction: account for the full set of operations and real time and space efficiency; workload "assume heavy"): the score becomes a **monthly cost** model in real units, replacing proxy weights chosen by hand.
- Benchmark change (`examples/bench.rs`, `docs/experiments/README.md`):
  - Per corpus, cost = stored bytes x storage price + the sum over operations of (times per month x (CPU time x CPU price + peak heap x wall time x memory price)).
  - Prices: $0.023 per GB-month, $0.04 per vCPU-hour, $0.005 per GB-hour (approximate cloud list prices).
  - Heavy workload `MONTHLY`, per corpus: ingest 1, repack 30, fsck 30, read passes 1,000, range reads 1,000 per file, searches 100,000 (100 per read pass, E050).
  - Score = geometric mean of kernel cost over baseline cost.
  - CPU time is process CPU across all threads (`clock_gettime(CLOCK_PROCESS_CPUTIME_ID)`, via `libc` as a dev-dependency already in the lockfile). Repack plans in parallel: one md repack used 15.5 s of CPU in 5.1 s of wall time.
  - Repeated operations run at least 5 times and 300 ms, and the median is used. With 3 runs, timer noise on sub-millisecond baseline operations left run-to-run spreads of up to 32% per corpus. With 5 runs and 300 ms it is 0.2–4.8% per corpus and 1.5% on the score.
  - Protocol: keep a change only if the score improves by more than the two-run spread on both runs. Sizes and deterministic proxies must still match exactly. `proxy_score` (E062), `read_score` and `bytes_score` are still reported.
- Result (two runs; every corpus exact with a clean fsck; every search result exact): **score 28.97 / 28.55**; proxy_score 1.5736.

  | corpus | kernel / baseline cost | kernel $/month | baseline $/month | kernel's largest parts |
  |---|---|---|---|---|
  | md | 26.4 | 0.0247 | 0.00095 | search 0.0192, repack 0.0051 |
  | pdf | 7.5 | 0.0026 | 0.00039 | search 0.0020, repack 0.0006 |
  | office | 65.1 | 0.59 | 0.0090 | search 0.55, repack 0.027 |
  | invoices | 44.7 | 0.0015 | 0.000034 | search 0.0012, repack 0.0003 |
  | synthetic-history | 5.7 | 0.0037 | 0.00087 | search 0.0032 |
  | repo-history | 9.4 | 0.150 | 0.0162 | search 0.117, repack 0.031 |
  | repo-graph | 83.0 | 0.00064 | 0.000008 | repack 0.0005, graph read 0.0001 |
  | generated | 160.6 | 0.44 | 0.0028 | search 0.43 (WASM) |
  | structured | 4.8 | 0.0239 | 0.0051 | search 0.0196, repack 0.0040 |
  | transcripts | 212.8 | 0.39 | 0.0018 | search 0.38 (WASM quote) |

  The ratios come from the second-iteration run (5 runs and 300 ms floor); the dollar figures from the first run (3 runs).
- What the model says:
  - **Storage is negligible under a heavy workload at list prices**: at most $0.00003 per corpus-month on either side. The kernel's smaller stores save about $0.00002 a month at most.
  - The kernel costs 5–213x the baseline, almost entirely in CPU time:
    - searches, which without an index decode everything, through WASM programs (generated, transcripts), zip rebuilds (office) and group prefixes (md, invoices);
    - daily repack, which costs more than all of the baseline's operations together on md, repo-graph, repo-history and office.
  - Under this objective the levers are: read speed (compile WASM to native rather than interpret it, avoid rebuilding containers on read, cheaper verification); search without decoding (E058's index); and the cost of repack itself. Further byte savings are worth nearly nothing at these prices.
- Verdict: **benchmark change kept** (the user's decision). Scores are not comparable with E062 or E034–E061.

### E065 search-index-decision
- Branch / parent: exp/065-search-index-decision / E064
- Hypothesis: under E064's heavy workload (100 searches per read pass), most of the kernel's cost is searches that decode every file. A trigram index (E058) cuts that on md, office, repo-history and generated. The kernel should decide for itself when to keep one, from a workload description.
- Change:
  - `src/repo.rs`: `Repo::search_rate` (searches per read pass, default 0) and `Repo::search_sample` (sample queries), set by the caller as `read_weight` is.
  - `src/search.rs`: `plan_search_index(files)` encodes an index in memory and measures the sample queries' decode work with it, and one query's without (a scan's work does not depend on the query). It stores the index only if its stored bytes + `read_weight` x rate x indexed work < `read_weight` x rate x scan work. That is deterministic and priced by the planner's own weight.
  - `encode_index` and `decode_index` are split out of `build_search_index` and `search_index`.
  - `src/repack.rs`: repack ends with `plan_search_index` over the reachable root files.
  - Test `index_is_kept_only_when_it_pays`.
  - `examples/bench.rs`: for file corpora, the kernel is told `SEARCH_RATE` and 20 sample queries from a different seed than the 100 scored ones, so the decision is not tuned on the scored queries.
- Result (two runs; every corpus exact with a clean fsck; every search result exact; deterministic fields identical): score 28.97 / 28.55 -> **18.59 / 18.52** (-36%, far beyond the 1.5% spread); proxy_score 1.5736 -> 1.3421.

  | corpus | index kept | bytes | decode per search | cost ratio |
  |---|---|---|---|---|
  | md | yes | 66,133 -> 177,959 | 2.14 M -> 0.44 M | 26.4 -> **8.4** |
  | office | yes | 165,802 -> 1,366,957 | 51.9 M -> 18.8 M | 65.1 -> **12.5** |
  | repo-history | yes | 907,329 -> 1,748,904 | 31.3 M -> 10.0 M | 9.4 -> **6.2** |
  | generated | yes | 4,213 -> 26,260 | 1.32 M -> 0.57 M | 160.6 -> **52.2** |
  | pdf, invoices, synthetic-history, structured, transcripts | no (does not pay) | unchanged | unchanged | unchanged within noise |

  The index's bytes cost nearly nothing at list prices (office's 1.2 MB index is about $0.00003 a month), while the searches it narrows were most of the cost.
- Verdict: **kept.**
- Remaining cost leaders:
  - transcripts (224x) and generated (52x): WASM interpretation;
  - repo-graph (81x): daily repack;
  - invoices (45x): every search decodes all, and every file holds the query.

### E066 wasm-speed
- Branch / parent: exp/066-wasm-speed / E065
- Hypothesis (E063/E064: WASM interpretation dominates transcripts' and generated's cost): the interpreter was built with wasmi's `portable-dispatch`, which its documentation describes as compiling universally "at the cost of execution performance". Tail-call dispatch (`auto-dispatch`) should run the same modules faster with the same outputs. A second cost was that every call built an engine and recompiled its module.
- Change:
  - `Cargo.toml`: wasmi features `["auto-dispatch", "deterministic"]` (was `portable-dispatch`).
  - `src/wasm.rs`: compiled modules are cached per process by content hash (compiling is pure); each call still instantiates fresh, with its own fuel.
  - Measured separately on generated and transcripts:
    - the cache alone changed read time by 0–16% (transcripts 359 -> 302 ms; generated unchanged);
    - `auto-dispatch` cut reads 2.2–2.6x (generated 392 -> 172 ms, transcripts 302 -> 135 ms) and halved repack.
- Note: fuel totals depend slightly on the dispatch mode. Function work (bytes + fuel/1000) moved by 17 units on generated (1,322,701 -> 1,322,684) and 100 on transcripts. Outputs are identical (every exactness gate passes), and stored functions use a tiny fraction of the 2^30 fuel budget. This is documented in `wasm.rs`.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; deterministic fields identical across runs): score 18.59 / 18.52 -> **15.57 / 15.84** (-15%).
  - transcripts: 223.8 / 220.0 -> **87.2 / 71.8**.
  - generated: 52.2 / 50.5 -> **32.4 / 31.8**.
  - Other corpora are within run-to-run noise, and their sizes are identical.
- Verdict: **kept.**

### E067 repack-unchanged
- Branch / parent: exp/067-repack-unchanged / E066
- Hypothesis: the workload repacks daily, but between ingests the store is exactly as the last repack left it. Repack is a fixed point (E060), so repacks 2–30 can only redo the same work and keep the same plan. A persistent fingerprint of what a repack depends on lets them return at once.
- Benchmark change (part 1): `MONTHLY` prices the first repack once plus 29 repacks of the store the first left (`repack_again`), each measured, instead of 30 times the first. Alone, with the E066 kernel: score 15.57 -> 15.41 (within noise). A second repack cost about as much CPU as the first: md 16.7 s, office 92.1 s, repo-history 93.9 s. repo-graph's was already cheaper (0.4 s against 1.5 s), so its ratio moved 84 -> 37.
- Kernel change (part 2), `src/repack.rs`:
  - `repack` starts by comparing `repack_fingerprint()` with the ref `meta/repacked`, and returns at once if they match. Otherwise it runs, then stores the fingerprint of the store it leaves.
  - The fingerprint is a BLAKE3 hash of every stored id, every ref except its own, and the planner's settings (`read_weight`, `search_rate`, `search_sample`, `proposed_order`). Stored encodings change only inside repack, so ids and refs capture every change between two repacks.
  - The ref holds the fingerprint itself, not a stored object; reachability and gc ignore ids they cannot read.
  - Test `repack_skips_an_unchanged_store`: a skipped repack changes nothing; a new commit changes the fingerprint and the next repack runs.
- Result (two runs with part 2; tests, clippy and fmt clean; every corpus exact with a clean fsck; deterministic fields identical): score 15.57 (E066) -> 15.41 (part 1 only) -> **10.79 / 10.82** (-30%).
  - A repack of an unchanged store now costs 0.07–0.9 ms of CPU (was 0.4–94 s).

  | corpus | cost ratio |
  |---|---|
  | md | 8.1 -> 2.7 |
  | office | 13.3 -> 10.3 / 10.9 |
  | invoices | 45.3 -> 34.4 / 35.1 |
  | repo-history | 5.9 -> 4.1 / 3.9 |
  | repo-graph | 84 -> 19 |
  | generated | 32 -> 28.5 / 28.8 |
  | pdf | 7.3 -> 5.5 |

  - synthetic-history, structured and transcripts: unchanged within noise (search-dominated).
- Verdict: **kept.**

### E068 prepared-dictionary
- Branch / parent: exp/068-prepared-dictionary / E067
- Hypothesis: every decode with the shared dictionary called zstd's `load_dictionary`, rebuilding its tables from about 100 KB each time, while the baseline prepares its dictionary once. Most md content and every group frame decode with the dictionary.
- Change, `src/dict.rs`:
  - Dictionaries are prepared once per process, per dictionary id (`prepared`, a zstd `DDict`), and decoding references the prepared one (`Dict::Prepared`).
  - Prefix decoding drives zstd's streaming API directly, because the reader adapter cannot borrow a prepared dictionary. Raw dictionaries (a `Q` encoding's base) still load per frame.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; deterministic fields identical across runs and to E067): score 10.79 / 10.82 -> **10.58 / 10.30**.
  - md 2.73 -> 2.43; invoices 34.4 -> 33.0; pdf 5.5 -> 5.3; repo-graph 19.0 -> 17.7.
  - generated (28.5 -> 34.2 / 28.3) and transcripts (92 -> 94 / 77): WASM-dominated, within their noise.
- Profile (callgrind, md reads after repack): reads are almost all zstd's own sequence decoding, of group prefixes. Each md file sits in a group, and reading it decodes the group up to its end. Dictionary loading was a small share, which is why this gain is small.
- Verdict: **kept** (a small gain).
- Next: the planner's `read_weight` (0.001 stored bytes per decoded byte) is far from what the cost model implies under the heavy workload, roughly 50. So it keeps trading reads for bytes, as groups do, where storage is nearly free.

### E069 read-weight-from-cost
- Branch / parent: exp/069-read-weight-from-cost / E068
- Hypothesis: E068's profile showed md reads spend their time decoding group prefixes. That is a storage-for-reads trade the planner makes because `read_weight` = 0.001 stored bytes per decoded byte. Under E064's heavy workload at list prices, one decoded byte per read costs about 1 ns of CPU ($1.1e-14), and a content is decoded about 101,000 times a month (1,000 read passes plus 100,000 searches without an index), against $2.3e-11 per stored byte-month. The rate the score implies is therefore about 49, not 0.001. Given that rate, the planner should stop trading reads for bytes.
- Change:
  - `examples/bench.rs`: `READ_WEIGHT` is derived from the cost model's constants (decodes per month x `DECODE_SECONDS_PER_BYTE` = 1 ns, from E063's baseline reads at 0.6–2.3 ns/B, x CPU price / storage price) = 48.8, and set as the planner's `read_weight`. The kernel's default stays 0.001.
  - `src/dict.rs`: `encode_cheapest` also considers raw storage (`B`), which it never did.
  - Raw reads are priced, not free:
    - measured here, zstd decodes at 2.2 ns/B, BLAKE3 verification costs 0.25 ns/B and copying 0.02 ns/B, so a verified raw read costs about 1/8 of a zstd decode;
    - `codec::read_weight(b'B')` = 1/8 (now `f64`); decode cost counts raw bytes at 1/8;
    - counting is in eighth-units, rounded per call (`count_decode`), so a read counts the same whatever ran before. A first version divided a running total and made the proxies differ by a few units between runs; that was caught by the determinism check and fixed.
  - With raw reads first priced at 0 (an intermediate version), the search-index decision saw scans as free and dropped office's index (office 0.41 -> 1.29). Pricing them at 1/8 restored it.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact; deterministic fields identical across runs): score 10.58 / 10.30 -> **0.78 / 0.76**. For the first time the kernel costs less than the baseline overall. bytes_score 0.45 -> 5.23: under this workload it stores about 5x the baseline's bytes, by choice.

  | corpus | cost ratio | bytes (zstd+dict) |
  |---|---|---|
  | md | 2.44 -> **0.30 / 0.27** | 177,959 -> 301,753 (76,892) |
  | pdf | 5.33 -> **0.17** | 20,853 -> 163,266 (24,643) |
  | office | 9.9 / 11.0 -> **0.24 / 0.25** | 1.37 M -> 4.08 M (0.39 M) |
  | invoices | 33 -> 3.70 | 3,662 -> 86,608 (13,519) |
  | synthetic-history | 4.9 -> 1.37 | 15,671 -> 662,387 (60,744) |
  | repo-history | 3.9 -> **0.39** | 1.75 M -> 6.14 M (1.29 M) |
  | repo-graph | 17.7 -> 11.6 | 18,348 -> 50,013 (113,441) |
  | generated | 34 / 28 -> **0.26** | 26,260 -> 714,745 (175,017) |
  | structured | 3.9 -> **0.91 / 0.85** | 0.10 M -> 8.93 M (1.00 M) |
  | transcripts | 94 / 77 -> 1.21 / 1.06 | 72,557 -> 3.42 M (0.32 M) |

  Plans now store most files raw (`B`). Zips are stored rather than re-deflated, generated's and transcripts' outputs are stored rather than recomputed by WASM, and groups and templates are gone. Search indexes are kept where they pay.
- What it means: under a heavy workload at list prices, the kernel's best plan for hot data is to store it raw, with verified reads and an index for search. Compression, templates, slices and programs pay only when reads are rare. The planner now takes that trade-off from the workload (`read_weight`), not a constant: with the default 0.001 it plans for space, as before.
- Verdict: **kept.**
- Remaining above 1.0:
  - invoices 3.7: per-file overhead on 30 tiny files, and every search scans all of them;
  - repo-graph 11.6: graph reads;
  - synthetic-history 1.37;
  - transcripts 1.1–1.2.

### E070 experiment-registry (process)
- Branch / parent: exp/070-experiment-registry / E069
- Why (the user's direction): the score has changed meaning three times, so many past verdicts may not hold any more. Which ones are still relevant, and which are worth rerunning, should be tracked, ideally in IKAM itself.
- Change:
  - `docs/experiments/registry.tsv`: one row per experiment (E000–E069), giving its kind, verdict, where it is recorded, the objective it was measured under, the mechanisms it touched, and whether its verdict depends on the storage-versus-read weights.
  - `examples/registry.rs` derives relevance (current, holds, stale, reference) and a rerun action for stale entries (ablate kept mechanisms, retest rejected ones), with a priority. It commits the registry as a versioned IKAM graph to a filesystem store (experiments, objectives, mechanisms and statuses as nodes; each log entry as content), reads the stale list back from the stored graph, and checks the store with fsck.
  - Workflow in `docs/experiments/README.md`, "Relevance".
- Result: under the cost objective, 5 experiments are current, 20 hold, 12 are benchmark references and 33 are stale.
  - Ablate, high priority (6): E022, E023, E026, E035, E037, E056.
  - Retest, high priority (8): E013, E014, E025, E029, E044, E046, E047, E061.
  - Low priority: 16 to ablate, 3 to retest (mostly compression, which is nearly free under this workload).
  - A rerun with an unchanged registry adds no new version.
- Verdict: **kept** (process; no kernel change).

### E071 contiguous-id-hash
- Branch / parent: exp/071-contiguous-id-hash / E070
- Hypothesis (callgrind of a search over 30 small raw files: 88% of the CPU is BLAKE3 verification, mostly `ChunkState::update`): `Id::of_content` fed BLAKE3 the 1-byte `"B"` prefix and then the content. That forces the first 1 KiB chunk through the byte-wise chunk path. Measured per byte:

  | content size | prefix then bytes | one buffer |
  |---|---|---|
  | 5 KiB | 0.91 ns | 0.49 ns |
  | 16 KiB | 0.43 ns | 0.20 ns |
  | 1 MiB | 0.18 ns | 0.28 ns (the copy costs more than it saves) |

- Change, `src/id.rs`: contents up to 256 KiB are hashed as one copied `"B" ++ bytes` buffer; larger ones as before. The ids are unchanged. Test `content_id_is_the_blob_id_at_every_size` covers both paths and their boundary.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; sizes and deterministic fields identical to E069): score 0.7804 / 0.7552 -> **0.7618 / 0.7552**.
  - invoices: 3.70 / 3.68 -> **2.86 / 2.83** (search 102 -> 76 µs), beyond its spread in both runs.
  - transcripts 1.21 / 1.06 -> 1.07 / 1.09. Every other corpus is within noise.
- Verdict: **kept, but it does not meet the score rule.** The score improves beyond the previous two-run spread on only one run; the other ties. It is kept as a pure speedup with no regression and a consistent per-corpus gain. Stated here so the exception is visible.

### E072 memmem-search (benchmark correction and kernel change)
- Branch / parent: exp/072-memmem-search / E071
- Hypothesis: both the kernel's `search` and the baseline's searched each content with a naive window comparison (O(n x m)). With content now stored raw (E069), that scan is a large share of search time on both sides. A real substring search (`memchr::memmem`, already a kernel dependency) speeds up both, and the comparison stays fair only if both use it.
- Change: `src/search.rs` uses a `memmem::Finder` built once per query. `examples/bench.rs` makes the baseline's search use the same. The exactness oracle keeps the naive scan, as an independent check.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact; sizes and deterministic fields identical to E071): score 0.7618 / 0.7552 -> **1.2412 / 1.2814**. **Not comparable as an improvement**: the baseline's searches got much faster.

  | corpus | search CPU per query, kernel | search CPU per query, baseline | cost ratio |
  |---|---|---|---|
  | office | 1.10 -> 0.30 ms | 8.21 -> 0.54 ms | 0.23 -> 1.94 / 2.09 |
  | md | 0.10 -> 0.10 ms | 0.85 -> 0.37 ms | 0.30 -> 0.66 / 0.68 |
  | synthetic-history | 0.74 -> 0.25 ms | 0.56 -> 0.07 ms | 1.40 -> 4.21 / 3.07 |
  | generated | 0.56 -> 0.22 ms | 2.49 -> 0.78 ms | 0.27 -> 0.44 |
  | pdf | 0.04 -> 0.02 ms | 0.33 -> 0.11 ms | 0.20 -> 0.40 |
  | repo-history | 4.08 -> 2.69 ms | 14.3 -> 11.7 ms | 0.36 -> 0.32 / 0.48 |
  | structured | 3.49 -> 3.26 ms | 4.50 -> 5.83 ms | 0.90 -> 0.67 / 0.87 |
  | transcripts | 1.52 -> 1.28 ms | 1.56 -> 1.29 ms | 1.08 -> 1.11 / 1.13 |
  | invoices | unchanged | unchanged | 2.85 -> 2.80 / 2.77 |

- What it shows:
  - Part of E069–E071's apparent lead came from the baseline's naive scan, not from the kernel.
  - With equal scanning, what separates kernel and baseline on raw-stored content is mostly the kernel's BLAKE3 verification of every read: 0.18–0.9 ns/B, against zstd decoding highly repetitive text at under 0.2 ns/B (synthetic-history: 650 KB decoded in 0.07 ms).
  - The baseline verifies nothing on read.
- Verdict: **kept** (a fairness correction that also speeds up the kernel). The score definition is unchanged, but values before E072 overstate the kernel against an equal-effort baseline.
- Open question for the user: should every read keep re-verifying, or should content be verified once when it is loaded from storage? See the report.

### E073 graph-read-baseline (benchmark correction)
- Branch / parent: exp/073-graph-baseline / E072
- Hypothesis: on repo-graph the comparison is unfair to the kernel. The kernel's graph read returns every version's `Vec<Node>`, but the baseline's "read" only decompressed the edge lists and never turned them into a graph. A fair baseline decompresses each edge list and then parses it with `graph_of`, the same function that builds the graphs the kernel stores. That yields the `Vec<Node>` the kernel's read returns, which the exactness gate already checks against `graph_of`.
- Change: in `examples/bench.rs`, `operation_profile`'s baseline read parses each decoded edge list with `graph_of` when the corpus is a graph. No kernel change. File corpora are untouched.
- Result (two runs; clippy and fmt clean; every corpus exact with a clean fsck; sizes and deterministic fields identical to E072): score 1.2412 / 1.2814 -> **0.9688 / 0.9938**. **Not comparable as an improvement**: only the baseline's repo-graph read changed.

  | repo-graph | E072 | E073 |
  |---|---|---|
  | baseline read, CPU per pass | 0.41 ms | 10.1 ms |
  | baseline read, peak heap | 1.0 MB | 3.0 MB |
  | kernel read, CPU per pass | 7.8–8.9 ms | 7.6–7.7 ms |
  | cost ratio | 13.17 / 11.82 | **0.774 / 0.770** |

  Other corpora moved only by run-to-run noise (md 0.48 / 0.67, the widest).
- What it shows:
  - The kernel's chunked graph read (332 KB decoded into nodes) is about 25% cheaper in CPU than decompressing and parsing the 1 MB of edge lists. It also has a smaller heap peak (0.19 MB against 3.0 MB).
  - repo-graph was the single largest term in the score. It now favours the kernel.
- Remaining corpora above 1.0: synthetic-history (4.2–4.4), invoices (2.8), office (1.9–2.1), transcripts (1.1–1.3). E072 named the cause: the kernel verifies every read with BLAKE3, and the baseline verifies nothing.
- Verdict: **kept** (benchmark correction). Score values before E073 overstate repo-graph's cost.

### E074 checked-raw (verification chosen per encoding)
- Branch / parent: exp/074-checked-raw / E073
- Hypothesis: E072 found that most of the kernel's remaining read cost on content stored raw (E069) is the BLAKE3 hash every read computes against the id (0.17–1.1 ns/B measured, against 0.012–0.044 ns/B for CRC-32 and 0.01–0.06 ns/B for the copy itself; microbenchmark on this machine, 512 B to 1 MiB). The user chose option 3: content stored as is is checked with a fast checksum, while decoded and derived content keeps BLAKE3. If the checksum belongs to the encoding, the planner can price it like any other decode cost.
- Change:
  - New plain encoding `"W" crc32_le[4] bytes` (`dict::checked_raw`).
  - A read checks the CRC, and `rebuild` skips the id hash for it (`dict::self_verified`). A damaged body reads as `Corrupt`, so the read falls through to derivations as before.
  - `encode_cheapest` offers it alongside "B". It costs 4 bytes more and is priced at a read weight of 1/64 of zstd, against 1/8 for "B". `DECODE_UNITS` went from 8 to 64 so that 1/64 counts in whole units; "B" counts are unchanged.
  - Repack and grouping treat "W" like "B".
  - fsck still hashes every content with BLAKE3.
  - `crc32fast` (already in the lockfile through flate2) is now a direct, pinned dependency.
  - New test `checked_raw_reads_check_their_crc`: damage is an error, and fsck catches bytes swapped under a matching CRC.
- What it does not protect against: bytes swapped on purpose, with a matching CRC, in the store. A read returns them until fsck runs (30 times a month under `MONTHLY`). Accepted by the user as option 3.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact; deterministic fields identical across runs): score 0.9688 / 0.9938 -> **0.6271 / 0.6061**. Stored bytes grew 4 B per "W" object, at most 0.44% (md, 301,753 -> 303,085). bytes_score 5.2310 -> 5.2350; proxy_score 3.4269 -> 3.4084.

  | corpus | E073 | E074 | kernel read pass CPU |
  |---|---|---|---|
  | invoices | 2.817 / 2.796 | **0.800 / 0.771** | 0.075 -> 0.013 ms |
  | synthetic-history | 4.179 / 4.363 | **1.689 / 1.662** | 0.236 -> 0.066–0.079 ms |
  | office | 2.089 / 1.867 | **1.661 / 1.632** | 1.31 -> 0.42 ms |
  | transcripts | 1.263 / 1.138 | **0.670 / 0.678** | 1.34 -> 0.72–0.75 ms |
  | structured | 0.890 / 0.883 | 0.522 / 0.497 | 3.48 -> 1.51–1.54 ms |
  | generated | 0.444 / 0.420 | 0.288 / 0.259 | 0.26 -> 0.11 ms |
  | pdf | 0.410 / 0.491 | 0.337 / 0.333 | 0.10 -> 0.02 ms |
  | repo-history | 0.392 / 0.386 | 0.265 / 0.226 | 3.71 -> 1.44–1.48 ms |
  | md | 0.477 / 0.669 | 0.604 / 0.630 (within E073's spread) | 0.33 -> 0.10 ms |
  | repo-graph | 0.774 / 0.770 | 0.770 / 0.771 (graph chunks are objects, still hashed) | unchanged |

- What it shows:
  - The planner chose "W" for every content it stores raw. "B" remains only for a 1-byte content in repo-history.
  - Two corpora are still above 1.0: office and synthetic-history (each about 1.65).
- Seen while reading the code, not changed here (one hypothesis per branch): for "Y", "Q", "X", "R" and "P", `decode_plain` checks the decoded bytes against the id, and then `rebuild` hashes them a second time. That is two BLAKE3 passes per read of compressed content.
- Verdict: **kept**. Improves beyond the two-run spread on both runs.

### E075 single-hash (one verification per read of compressed content)
- Branch / parent: exp/075-single-hash / E074
- Hypothesis: for "Y", "Q", "X", "R", "P", "K" and "S", `decode_plain` already checks the decoded bytes against the id. That check picks the right dictionary or base, or checks a group member's own id. `rebuild` then hashed the same bytes again. Skipping the second hash saves one BLAKE3 pass per read of compressed content, with the same guarantee.
- Change: `dict::self_verified` names those encodings as well as "W", and `rebuild` does not hash their output again. Only "B" and "Z" come back from `decode_plain` unverified. New test `self_verified_encodings_reject_other_content`: well-formed frames of another content, stored under an id, read as `Corrupt` for every codec in the table.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact; sizes and every deterministic field identical to E074): score 0.6271 / 0.6061 -> 0.6076 / 0.5900. **Not beyond the two-run spread on both runs**: run 1 (0.6076) is above E074's better run (0.6061). Read CPU per pass is unchanged within noise on every corpus. As expected, under the E069 read weight the hot content is stored raw ("W"), and compressed content is rarely read in the workload.
- Verdict: **kept**, stated as not a measured improvement (as with E071). It removes redundant work without changing what a read guarantees.

### E076 frame-checksum (zstd's own check per frame)
- Branch / parent: exp/076-frame-checksum / E075
- Plan change, stated before the work: E074's report proposed "proofs at admission plus leaf checks, with explicit promotion" (a decoder-version fingerprint, recorded by repack). Reading the code showed a simpler mechanism. A zstd frame can carry its content checksum, which zstd checks against the decoded bytes on every whole read. The checksum is computed over the bytes that were encoded, and every writer has hashed those bytes into the id. So each frame binds itself to its id when written and re-checks the reconstruction on every read, whatever decoder version runs. No proof store or version fingerprint is needed for zstd frames. Promotion and fingerprint machinery would matter only for encodings that cannot check themselves (derivations, i.e. program outputs). It was not built, because nothing measured shows it would pay (see below).
- Hypothesis: frames that repack keeps carry zstd's checksum (4 B each). Reads of "Z", "Y" and "Q" accept it instead of BLAKE3, the same pattern as E074's "W".
- Change:
  - Frames written at `STORE_LEVEL` carry the checksum. Block frames ("K") do not, because blocks are read in part.
  - `frame_checked` reads the header bit. `self_verified` includes checksummed "Z". Decoding "Y" and "Q" skips the id check when the frame is checksummed, and the checksum also rejects a wrong dictionary or base.
  - `related_base` no longer re-hashes a verified base.
  - fsck still hashes everything.
  - New test `checksummed_frames_are_checked_by_zstd`.
- Result (two runs; tests and clippy clean; every corpus exact with a clean fsck; every search exact): score 0.6076 / 0.5900 -> 0.5983 / 0.6026, **within noise**. Read CPU per pass is unchanged on every corpus. Stored bytes went up by 4–393 B per corpus: tree objects' frames also got checksums they never use.
- What it shows: under the E069 read weight, hot content is stored raw ("W"), so compressed frames are rarely read in this workload. The same reasoning says a derivation-proof step (the "E077" proposed in the report) would not pay now: reads build almost nothing through derivations. Where the kernel's monthly cost now goes (E076 run 1, share of kernel cost):

  | corpus | ratio | top costs |
  |---|---|---|
  | office | 1.60 | repack 73%, search 14%, storage 10% |
  | synthetic-history | 1.68 | search 58%, repack 29%, storage 11% |
  | md | 0.67 | repack 62%, search 33% |
  | transcripts | 0.66 | search 80% |
  | structured | 0.53 | search 71%, repack 20% |
  | repo-graph | 0.50 | graph read 96% |

  The baseline's cost is 96–98% search on every file corpus. So the kernel's remaining levers are the first monthly repack and search, not verification.
- Verdict: **discarded** (neutral, with more bytes). Kept on its branch. It would pay under a workload that reads compressed content often, for example a colder read weight or an archive profile.

### E077 cross-source (analysis: one graph against one store per file type)
- Branch / parent: exp/077-cross-source / E076 (main at 03fa78a)
- Question: does storing different file types in **one** graph let the kernel share information across them (the north star in `docs/plans/2026-10-06-system-architecture.md`)? The bench cannot answer this: it stores md, pdf and office as separate corpora.
- Method:
  - New `examples/cross.rs` (an analysis, not scored). Each corpus is stored once as one repository and once as one repository per file type, then repacked; the kernel's stored bytes are compared. The baseline (zstd 3 with a trained dictionary) is split and combined the same way.
  - Exactness and fsck are asserted.
  - Two corpora:
    - the fixture cases' documents: md, pdf, docx/xlsx/pptx, json, py, 4.9 MB; images left out;
    - new `fixtures/multiformat`: each of the 24 cases' `idea.md` also rendered to html and docx by pandoc 3.1.3, and to pdf (docx through LibreOffice 24.2), 2.9 MB. The same text in four formats.
  - Literal overlap in multiformat (16-byte shingles of the md text found verbatim): html 43%, docx `word/document.xml` (inflated) 46%, pdf content streams (inflated) 0%. The PDF text is glyph-coded.
- Result at read weight 0.001 (storage only):

  | corpus | kernel per type | kernel one store | baseline per type | baseline one store |
  |---|---|---|---|---|
  | cases | 385,960 | **400,092 (+3.7%)** | 797,709 | 870,159 (+9.1%) |
  | multiformat | 881,988 | **894,449 (+1.4%)** | 1,123,952 | 1,125,583 (+0.1%) |

  At the bench's read weight (48.8) on cases, nearly everything is stored raw: 4,853,079 split and 4,827,003 one store (-0.5%), against the baseline's 797,709 and 870,159. Under the heavy workload, storage hardly matters, so that weight cannot show compression.
- What it shows:
  - The kernel already stores both corpora at 0.48x and 0.78x of the baseline.
  - **One graph is worse than separate stores.** The opposite of the goal.
  - A zstd-only probe (Python zstandard, level 3, no dedup) on the cases' stored samples (office members inflated), against 613,904 B for one dictionary capped at 112,640 B as now:
    - one larger dictionary of about 300 KB: 578,637 B (-5.7%);
    - one dictionary per file type, used only where it pays: 534,198 B (-13%);
    - two dictionaries, samples reassigned to the one that compresses them best (no file types given): 534,779 B (-12.9%, the same as knowing the types);
    - three or more clusters did not converge with this simple reassignment.
  - multiformat's bytes are 87% pdf (763,683 kernel B). The kernel expands zip members but not PDF streams, so its fonts and content stay deflated and opaque.
- Next hypotheses, one per branch: E078 larger dictionaries; E079 several dictionaries by clustering; E080 PDF streams expanded like zip members; then cross-format context (md as the zstd context of its html and docx).
- Verdict: **reference** (analysis; adds `examples/cross.rs` and `fixtures/multiformat`, 3.0 MB).

### E078 larger-dictionary (dictionaries up to 1 MiB)
- Branch / parent: exp/078-larger-dictionary (not kept; a one-line change) / E077
- Hypothesis: E077's probe estimated one 300 KB dictionary 5.7% smaller than the 112,640 B cap on the cases' mixed samples. Raise the cap of the dictionary sizes tried (`total / 100, / 30, / 10`) to 1 MiB; each size is still chosen only if the re-plans find it smaller.
- Result (`examples/cross.rs`, read weight 0.001):
  - cases: unchanged (one store 400,092 B). The planner still chose a 41,888 B dictionary for the mixed store, against 158,618 B of dictionaries across the five per-type stores.
  - multiformat: one store 894,449 -> 890,702; pdf store 763,683 -> 769,884.
- What it shows: the limit is one dictionary serving five kinds of content, not its size.
- Verdict: **discarded** (mixed, within 0.7%). Not benchmarked: the north-star measurement already refutes it.

### E079 dictionary-clusters (a dictionary per cluster of content)
- Branch / parent: exp/079-dictionary-clusters / E077
- Hypothesis: a store of mixed content needs several dictionaries. Change:
  - Repack clusters the samples in two, split first by their first bytes, then reassigned to the dictionary (zstd-trained per round) that compresses them best, for 3 rounds.
  - Each cluster's dictionary is chosen as the single one is.
  - The clustered plan is offered only if estimated more than 3% smaller.
  - Further dictionaries live under a new ref `meta/zstd-dicts` (a content listing their ids). `compress` tries each, and a "Y" read finds them by prefix, cached.
  - Grouping, Q bases, recompress, prune and ingest's deletes all treat every dictionary as the current one.
  - New test `further_dictionaries_encode_and_decode`.
- Result (`examples/cross.rs`, read weight 0.001):
  - cases office store 162,293 -> 167,591 (+3.3%), data 51,736 -> 53,194; split total 385,960 -> 392,716.
  - multiformat office 62,608 -> 55,024 (-12%).
  - **Both one-graph stores unchanged.** Their clustered estimate (643,540) never beat the best single dictionary's (632,616).
  - Earlier rounds with raw (untrained) dictionaries formed worse clusters (estimates up to 30% worse).
- What it shows: two-way clustering does not match five kinds of content, and the zstd-only estimate (no dedup, level 3) misleads the choice. A cluster count chosen by measured gain (bisecting while it pays) is untested.
- Verdict: **discarded** (does not reach its hypothesis; mixed per store). Code on its branch.

### E080 zlib-streams (deflate streams inside any file expanded)
- Branch / parent: exp/080-zlib-streams / E077
- Hypothesis: multiformat's bytes are 87% PDF, and the kernel expands zip members but not the zlib streams inside other files. A probe (`docs/research/recompression.md`) found that 255 of the 24 PDFs' 275 FlateDecode streams re-deflate exactly with zlib, and that the inflated streams compress solid to 138,390 B (from 843,494 B deflated). Expanding them as members of the existing `deflate-pack` function should shrink the PDFs and let shared fonts and content dedup across files.
- Change:
  - New `container::plan_zlib`: a valid zlib header, a deflate stream that ends, and the Adler-32 after it matching the inflated bytes (at least 64 B). Each stream that re-deflates exactly at some zlib level becomes a member. Header and checksum stay literal.
  - `plan_zip` and `plan_zlib` share one manifest builder. Ingest tries `plan_zip`, then `plan_zlib`.
  - No new function: `deflate-pack` already rebuilds "literal segments + deflate at a recorded level".
  - No file type is named.
  - New test `zlib_streams_expand_and_rebuild`.
- Result:
  - `examples/cross.rs` (read weight 0.001), multiformat:

    | | before | after |
    |---|---|---|
    | pdf store | 763,683 | **437,094 (-43%)** |
    | split total | 881,988 | 555,399 |
    | one store | 894,449 | **554,151 (-38%)** |
    | one store vs baseline | 0.79x | **0.49x** |

    One graph is now 0.2% smaller than the per-type stores (was 1.4% larger). The cases corpus is unchanged: its PDFs (reportlab) have no deflate streams.
  - Bench (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every stored size and deterministic field identical to E075): score 0.6181 / 0.6001 against 0.6076 / 0.5900, within noise. No bench corpus holds a planable zlib stream: office goes through `plan_zip`, and images are not in the bench. Ingest time unchanged.
- Verdict: **kept** (neutral on the bench, -38% on the north-star corpus).

### E081 multiformat-corpus (benchmark change: the north-star corpus is scored)
- Branch / parent: exp/081-multiformat-corpus / E080
- Change: the bench adds corpus `multiformat` (`fixtures/multiformat`, 96 files of the same 24 texts in md, html, docx and pdf), stored as **one** graph. It is the first scored corpus that mixes file types in one store. `examples/cross.rs` prints each store's bytes by encoding to stderr.
- Result (two runs; every corpus exact with a clean fsck; the other ten corpora's sizes and deterministic fields identical to E080): score over **11** corpora 0.5717 / 0.5788. Not comparable with earlier scores, which are over 10.
  - multiformat cost ratio 0.374 / 0.381. Under the heavy workload, the kernel stores all 96 files raw ("W", 2,879,743 B) plus a search index (2,916,522 B), 5,797,532 B in all against the baseline's 1,125,583 B. Search is 99% of the baseline's monthly cost (1,000 read passes x 100 searches over 2.9 MB).
  - Its first repack costs 43–47 s of CPU at a 124 MB heap peak, 73–78% of its kernel cost.
- What it shows: under the heavy, uniform workload, storage is about 15% of the kernel's cost on this corpus, so the score barely sees compression or cross-type sharing. Progress on the north star is therefore also measured by `examples/cross.rs` at read weight 0.001 (storage only), reported beside the score.
- Verdict: **kept** (benchmark change; reference for the 11-corpus score).

### E082 solid-groups (larger groups, priced by reads)
- Branch / parent: exp/082-solid-groups (not kept) / E081
- Hypothesis: multiformat's PDF store (437,094 B after E080) is about twice a solid zstd 19 of the same expanded content (217,863 B, Python probe). Its 24 similar manifests and its fonts are compressed one object at a time. Groups are capped at 64 KiB, but `plan_group` already prices group reads (decode up to the member's end) by the read weight, so larger groups could be left to the planner.
- Change tried: the seed proposers also build groups of up to 4 MiB, then 256 KiB and 1 MiB; contents up to 4 MiB were eligible. The measured proposers were unchanged.
- Result (`examples/cross.rs`, read weight 0.001):
  - 4 MiB: one group per store, rejected by `plan_group` every time. Reading each member decodes the group up to it: n^2/2 x the member size, about 500 MB for 349 members, which at weight 0.001 outweighs any saving.
  - 256 KiB and 1 MiB: pdf 437,094 -> 441,068 and office 62,608 -> 62,017. Within noise; the one store unchanged.
- Verdict: **discarded**. Under the read model, solid context across large contents does not pay as groups.

### E083 deflate-memlevel (zlib memLevel 9 streams reproduced)
- Branch / parent: exp/083-deflate-memlevel / E081
- Hypothesis: of multiformat's 275 PDF streams, the 19 that `plan_zlib` left literal (195,945 B deflated, the largest: fonts) all re-deflate exactly with zlib at level 6 and **memLevel 9** (Python probe over memLevel 1–9 and strategies 0–4; the other 256 are memLevel 8). flate2 fixes memLevel at 8. Reproducing memLevel 9 should expand them too.
- Change:
  - `deflate` takes manifest levels 0–19. Levels 10–19 are zlib levels 0–9 at memLevel 9, through the pinned zlib directly (`libz_sys::deflateInit2_`). This is the kernel's first `unsafe`: a 30-line wrapper with zlib's own allocators.
  - The format is backward compatible: `pack` rejected levels above 9 before, so no stored manifest holds one.
  - `deflate_output_is_pinned` now pins all 20 outputs (`GOLDEN_MEM9`) and checks each inflates back.
  - Each level is tried at memLevel 8 first, then 9.
- Result:
  - `examples/cross.rs` (read weight 0.001), multiformat:

    | | before | after |
    |---|---|---|
    | pdf store | 437,094 | **268,472 (-39%)** |
    | one store | 554,151 | **382,635 (-31%)** |
    | one store vs baseline | 0.49x | **0.34x** |

    That is within 11% of a solid zstd 19 (long) over all the expanded content (345,208 B). One graph is 1.1% smaller than the per-type stores. The cases corpus is unchanged.
  - Bench (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every stored size identical to E081): score 0.6120 / 0.5858 against 0.5717 / 0.5788. Within noise, with a wider spread than usual in this pair of runs (repo-graph 0.70–1.06 on unchanged bytes). Ingest and repack times are within run-to-run variation, office included.
- Verdict: **kept** (neutral on the bench, -31% on the north-star corpus).

### E084 two-base-context ("M": a shape context and a words context)
- Branch / parent: exp/084-two-base-context / E083
- Hypothesis: a content in one format shares its *structure* with other contents of that format and its *words* with the same text in another format. A probe (Python zstandard, level 19, raw-content dictionaries; multiformat) measured both together, against the same-format context alone:

  | content | alone | same-format neighbour | own md | both |
  |---|---|---|---|---|
  | html | 84,067 | 35,607 | 52,092 | **12,781** |
  | docx `word/document.xml` | 70,436 | 40,992 | 38,361 | **17,219** |

  A "Q" frame has one context, so two contexts should capture information shared across formats.
- Change:
  - New plain encoding `"M" ref[32] ref[32] frame`: zstd with both contents (in order) as a raw-content dictionary. Bases are as for "Q": never "Q", "M" or "S", so decoding goes one step.
  - Base choice: the first base is the most related (shingles), as before. The second is `Bases::complement`, the candidate sharing the most of the content's shingles that the first lacks (a greedy cover). No format is named.
  - Recompress prices Q and M like every encoding (bytes + read weight x the bytes decoded, contexts included) and keeps the cheapest. It now skips building a frame whose read cost alone exceeds the current encoding's cost: exact pruning, same choices.
  - `needs` returns every prefix an encoding needs. gc, ingest's base guard and `is_related_base` cover both contexts.
  - New test `two_contexts_decode_and_are_kept`.
- Result:
  - `examples/cross.rs` (read weight 0.001):

    | | before | after |
    |---|---|---|
    | cases, one store | 400,092 | **391,496 (-2.1%; one graph now 1.4% above split, was 3.7%)** |
    | multiformat, one store | 382,635 | **377,488 (-1.3%; 0.34x the baseline)** |

  - Bench (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every stored size identical): score 0.5829 / 0.5924 against E083's 0.6120 / 0.5858, within noise.
    - Without the pruning, repack CPU rose 5–15%.
    - With it, repack CPU is equal or lower: transcripts 6,871 -> 5,795 ms, invoices 795 -> 625 ms. Under the heavy read weight, most contexts are ruled out before compression.
- What it shows: much less than the probe, because most html and md contents are members of groups ("S"), and recompress never considers Q or M for group members. Grouping is decided before, and by bytes alone. Next: let grouping compete with the context encodings.
- Verdict: **kept**.

### E085 context-before-groups (keep context-encodable contents out of groups)
- Branch / parent: exp/085-context-before-groups (not kept) / E084
- Hypothesis: E084's "M" rarely applied because grouping comes first: in multiformat's one store, all 24 html and all 24 md files were group members ("S"), which recompress never gives a context. Before grouping, reserve (keep plain) each content whose Q or M estimate (MEASURE_LEVEL, read-priced) costs under 0.4 of its cost alone, together with the contexts it would use.
- Result (`examples/cross.rs`, read weight 0.001):
  - The reserve worked as intended per file. In the one store, 11 html and 12 md files became "M", at 300–1,400 B each, against about 9 KB of html raw. Many contexts were another format's content: docx XML and pdf members holding the same words.
  - Totals got worse: multiformat one store 377,488 -> 378,638, cases one store 391,496 -> 396,922. Office split 162,273 -> 158,035.
  - The groups left behind lost members that gave them context, and the reserved contexts must stay plain (no chains).
- What it shows: the context graph has to be chosen jointly (which contents serve as plain hubs, which are encoded against them), not greedily in store order with a fixed threshold.
- Verdict: **discarded**.

### E086 shared-container-plans (container plans computed once per repack)
- Branch / parent: exp/086-shared-container-plans (not kept) / E084
- Hypothesis: profiling office's repack with timers inside the 12 parallel re-plans attributed 4–8.6 s of each re-plan's ingest to container planning (inflate plus the deflate level search, which E083 doubled). All 12 re-plans compute the same plans.
- Change tried: a per-repack cache of `plan_zip` / `plan_zlib` results by content id, shared by the re-plans. First filled lazily, then precomputed in parallel before the re-plans start.
- Result (bench, two runs each; stored bytes identical): office repack CPU 61.3 / 63.8 s -> 57.6 / 60.3 s (lazy) and 62.2 / 62.2 s (precomputed). No gain beyond noise.
- What it shows: the timers were wrong. Twelve threads on 4 cores inflate per-thread wall time about 3x. Re-profiled with the re-plans run one at a time, each office re-plan takes 4–5.4 s: ingest 3.2–4.1 s, mostly members' slice and template candidates, which differ per re-plan. Container planning itself is small.
- Verdict: **discarded**.

### E087 fewer-replans (one dictionary re-planned, not five)
- Branch / parent: exp/087-fewer-replans / E084
- Hypothesis: the sequential profile (E086) shows every corpus runs 12 trial re-plans: 5 dictionaries (the 4 best estimates plus the best of each construction) and none, each with and without template induction. Each re-plan re-ingests the whole store. They are most of repack: office 56 of 64 s, repo-history 60 of 80 s, md 12 of 13 s. Repack is the largest kernel cost under the score. Re-plan only the best-estimated dictionary (and none), 4 re-plans instead of 12, and accept that the estimate (which ignores dedup) sometimes ranks wrong.
- Change: `DICT_SHORTLIST` 4 -> 1, and `train_dictionaries` no longer adds the best of each construction.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact): score 0.5829 / 0.5924 -> **0.4287 / 0.4930**. Improved beyond the spread on both runs.

  | corpus | repack CPU | cost ratio | stored bytes |
  |---|---|---|---|
  | office | 61.3 / 63.8 -> 25.1 / 24.8 s | 1.62 / 1.75 -> **0.65 / 0.93** | = |
  | repo-history | 84.9 / 84.6 -> 34.6 / 35.3 s | 0.22 / 0.23 -> 0.12 / 0.19 | -20,794 |
  | multiformat | 42.4 / 41.9 -> 19.3 / 19.0 s | 0.36 / 0.38 -> 0.24 / 0.24 | -7,853 |
  | md | 13.3 / 14.8 -> 4.7 / 4.7 s | 0.62 / 0.63 -> 0.36 / 0.42 | = |
  | pdf | 2.2 / 1.9 -> 1.1 / 1.3 s | 0.44 / 0.34 -> 0.27 / 0.27 | -948 |
  | structured | 45.5 / 44.2 -> 41.0 / 39.4 s (recompress dominates) | 0.51 / 0.46 -> 0.45 / 0.51 | -1,162 |

  Fewer candidates gave *smaller* stores on four corpora: the re-plans are compared at the fast measuring level, and the plan that wins there is not always smallest after recompress at the store level. bytes_score 5.2273 -> 5.2222.
- `examples/cross.rs` (read weight 0.001): one-graph stores unchanged (cases 391,496, multiformat 377,488). Per-type stores slightly larger (code 89,045 -> 92,499, md 63,305 -> 65,593), so the measured cross-type saving is now 0.0% and 2.0%.
- Remaining above 1.0: synthetic-history only (1.65 / 1.61).
- Verdict: **kept**.

### E088 prune-codecs (no codec run whose decode cost alone loses)
- Branch / parent: exp/088-prune-codecs / E087
- Hypothesis: after E087, recompress dominates some repacks (structured: 38 of 41 s). Under the heavy read weight, hot content ends up stored raw ("W"), yet `encode_cheapest` first compresses it with zstd at the store level (19) and brotli, and repack tries block frames for groups. A candidate's read cost (read weight x its decode weight x the bytes it decodes) is a lower bound on its total cost. When that bound already exceeds the best candidate's cost, the candidate cannot win, so it need not be built.
- Change:
  - `encode_cheapest` now evaluates the raw encodings ("B", "W") first, then builds zstd (`encode_plain`) and each active codec only if its decode-cost floor is below the best so far.
  - The block ("K") trials in recompress skip a block size the same way.
  - The pruning is exact: a skipped candidate could only have lost.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact; **every stored size and deterministic field identical to E087**):
  - Score 0.4287 / 0.4930 -> 0.4323 / 0.4149. Run 1 is not below E087's better run, so this is **not beyond the spread on both runs**.
  - Repack CPU:

    | corpus | before | after |
    |---|---|---|
    | structured | 41.0 / 39.4 s | **4.4 / 4.2 s** |
    | repo-history | 34.6 / 35.3 s | 20.5 / 19.3 s |
    | transcripts | 5.8 / 6.1 s | 2.7 / 2.7 s |
    | generated | 3.4 / 3.7 s | 1.8 / 1.8 s |
    | office | 25.1 / 24.8 s | 18.7 / 18.6 s |
    | synthetic-history | 2.5 / 2.3 s | 0.9 / 1.2 s |
    | multiformat | 19.3 / 19.0 s | 13.8 / 12.6 s |

    The whole bench ran in 38.7 / 38.4 s, against 109.3 / 107.5 s.
- Verdict: **kept** (identical output, much less CPU; the score's noise hides it, as with E071 and E075).

### E089 chunk-search (search what is shared once)
- Branch / parent: exp/089-chunk-search / E088
- Hypothesis: after E087–E088, search is the largest kernel cost on most corpora (62–85%), and several corpora are versions of the same data. A search that scans each distinct content-defined chunk once, plus each distinct seam between neighbouring chunks, does work in proportion to the distinct bytes rather than the total. Exact by construction: every chunk but a content's last is at least `MIN_CHUNK` (256 B) and longer patterns fall back to a scan, so a match lies inside a chunk or across one seam.
- Change (`src/search.rs`):
  - A second index kind next to trigrams, stored under the same ref. A gear-hash content-defined chunking (256 B to 8 KiB, about 1 KiB average). Each distinct chunk is owned by the largest content holding it.
  - Contents that own under a quarter of their own bytes (a version's few new chunks) have those chunks kept inside the index, so they are not read at search time.
  - When loaded, the distinct seams and the contents behind each chunk and seam are derived. A query scans each once and marks the contents behind each hit.
  - Planning picks none, trigrams or chunks by the existing cost (stored bytes + read weight x search rate x measured work). Two planning fixes:
    - **Scans are now priced.** Each scanned byte counts 2/64 of a zstd byte (memmem measured at about 0.08 ns/B against zstd's 2.2). Before, scanning content stored as is cost the planner almost nothing.
    - A chunk index must also beat the alternative by 25% (`CHUNK_MARGIN`). On transcripts the work units estimated chunks 5% cheaper than trigrams, while measured they were 16% slower; elsewhere both estimate and measurement put chunks 75–90% cheaper.
  - New test `chunk_search_matches_a_full_scan`: patterns in chunks, across every seam of a version, longer than a chunk, and in uncovered contents.
- Intermediate runs (recorded, not kept):
  - Without scan pricing, no chunk index was ever chosen.
  - With scans priced on only one path (a partial edit), chunk indexes were chosen where they were slower: invalid, rerun.
  - Without inlining, every synthetic-history version owned its new tail and was read in full: search did not speed up.
  - Without the margin, transcripts went 0.65 -> 1.0.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact): score 0.4323 / 0.4149 -> **0.3465 / 0.3595**. Beyond the spread on both runs.

  | corpus | search CPU per query | cost ratio |
  |---|---|---|
  | synthetic-history | 0.069 -> 0.022 ms | 1.36 / 1.41 -> **0.65 / 0.70** |
  | structured | 1.54 -> 0.31 / 0.35 ms | 0.43 / 0.38 -> **0.13 / 0.14** |
  | transcripts (trigrams kept) | 0.73 -> 0.70 ms | 0.65 / 0.67 -> 0.52 / 0.62 (noise) |

  Index bytes: synthetic-history +1,427 B, structured +3,560 B (its trigram index replaced). **For the first time, no corpus costs more than the baseline.** The highest is office at 0.80 / 0.82.
- What it shows: dedup pays in compute as well as storage. Search work follows distinct bytes once the shared structure is known.
- Verdict: **kept**.

### E090 three-replans (no dictionary re-planned once, not twice)
- Branch / parent: exp/090-two-replans / E089
- Hypothesis: of E087's 4 re-plans (the best dictionary or none, each with and without template induction), the no-dictionary plans rarely win. Dropping them cuts repack CPU again.
- Tried first: no dictionary re-planned only when no dictionary is estimated to pay (2 re-plans).
  - Repack CPU fell 20–45%.
  - But repo-graph lost the plan its graph reads need: stored bytes fell 50,013 -> 20,296, yet graph reads got slower, cost ratio 0.67 / 0.69 -> 1.07 / 0.79. md grew 999 B, and the score (0.3530 / 0.3217) was not beyond the spread.
- Kept: no dictionary re-planned without induction only, so 3 re-plans (4 when no dictionary pays).
- Result (two runs; tests and clippy clean; every corpus exact with a clean fsck; every search exact; **every stored size and deterministic field identical to E089**: the dropped plan never won):
  - Score 0.3465 / 0.3595 -> 0.3393 / 0.3466. Run 2 is 0.0001 above E089's better run, so not beyond the spread on both runs.
  - Repack CPU: office 18.7 -> 15.5 s, repo-history 20.4 -> 17.3 s, multiformat 11.6 -> 9.8 s, md 4.0 -> 3.4 s, generated 1.9 -> 1.5 s.
- Verdict: **kept** (identical output, less CPU).

### E091 groups-vs-contexts (dissolve a group when contexts store its members cheaper)
- Branch / parent: exp/091-groups-vs-contexts / E090. The joint choice of shared contexts against groups, approved by the user after E085.
- Hypothesis: E085 failed because grouping is decided in the re-plan, by bytes, before contexts exist, and its reserve heuristic was greedy. Instead, after recompress has given every plain content its cheapest encoding (Q and M included), revisit each group:
  - price each member at its cheapest alone (plain, or against contexts, excluding the group itself);
  - dissolve the group if the members cost less than the group plus its entries, read cost included;
  - dissolved members stored plain become contexts for later groups.
- Change: `context_encoding` (the Q/M trial factored out of recompress, same behaviour), `dissolve_groups`, `group_of`. A first version let a member take its own group as context, then deleted the group: the graph law test caught it.
- Result:
  - Bench (two runs; every corpus exact with a clean fsck; every search exact; **every stored size identical to E090**: under the heavy weight no group is dissolved): score 0.3503 / 0.3598 against 0.3393 / 0.3466, within noise.
  - `examples/cross.rs` at read weight 0.001: multiformat one store 377,488 -> 370,139 (-1.9%); cases one store 391,496 -> 398,481 (+1.8%).
  - Instrumented on the cases' PDF store: a group of 35 members (13,343 B stored) was dissolved for +1,035 B, because reading it costs 1.1 MB of decoding per read pass, which outweighs the bytes even at 0.001. So the weight-0.001 numbers include reads. The docs called them "storage only"; corrected in README.
  - At weight 0 (bytes alone), E091 changes almost nothing: multiformat one store 369,519 -> 369,040, cases identical. At weight 0, one graph is already smaller than the per-type stores: cases 352,338 against 358,181 (-1.6%), multiformat 369,519 against 380,717 (-2.9%).
- What it shows: for bytes, groups already capture most of what a second context would give, and dissolving them trades bytes for reads.
- Verdict: **discarded** (neutral on the bench and on bytes). Code on its branch.

### E092 futile-plans (no slice or template search when its read work alone loses)
- Branch / parent: exp/092-futile-plans / E091's parent (main at c21e308)
- Hypothesis: under the cost score almost every content ends up stored raw. The bench's stores hold no slice or template plans (concat/fill derivations), only "W", a few "Y"/"Z", and the programs and quote facts. Yet ingest and every re-plan search every content for slices and templates, and one re-plan in three exists only for template induction. A slice or template plan rebuilds the whole content, so its read work is at least the content's length. At read weight w, its cost is at least w x len, and ingest keeps a candidate only if its cost is below the plain encoding's length. When w x len >= the plain length, no such plan can win.
- Change:
  - Ingest skips `slice_candidate` and `template_candidates` when `read_weight x len >= plain length`. Container candidates (which expose members as content) and hints (facts) are unchanged.
  - Repack skips the induced re-plans when `read_weight >= 2`: a plain encoding is at most one byte longer than its content, so no plan can win then.
  - Both are exact pruning.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact; **every stored size, ingest size and deterministic field identical to E091**): score 0.3503 / 0.3598 -> **0.3221 / 0.3032**. Beyond the spread on both runs.

  | corpus | ingest CPU | repack CPU | cost ratio |
  |---|---|---|---|
  | office | 2,116 -> 686 ms | 11.9 -> 4.6 s | 0.49 / 0.75 -> **0.43 / 0.42** |
  | multiformat | 1,157 -> 292 ms | 8.1 -> 3.1 / 3.5 s | 0.18 / 0.20 -> 0.15 / 0.15 |
  | repo-history | 670 -> 126 ms | 14.4 -> 8.6 / 8.1 s | 0.15 / 0.14 -> 0.16 / 0.14 |
  | md | 88 -> 11 ms | 2.7 -> 1.6 s | 0.35 / 0.34 -> 0.30 / 0.18 |
  | structured | 329 -> 31 ms | 3.3 -> 2.3 s | 0.17 / 0.14 -> 0.15 / 0.14 |

  The bench ran in 20.7 / 20.5 s, against 31.4 / 31.6 s.
- What it shows: under the heavy workload, most of the planner's search was for plans the cost model could never pick. Pricing them before searching removes that work without changing a byte. At low read weights (archives, `examples/cross.rs`) every candidate is still searched.
- Verdict: **kept**.

### E093 read-priced-objects (graph chunks priced by their reads)
- Branch / parent: exp/093-read-priced-objects / E092
- Hypothesis: repo-graph's cost is 96% graph reads (6.85 ms per read pass against the baseline's 8.84 ms). Instrumented, each graph-chunk read splits into about 20 us decompressing the object frame, 18 us parsing nodes and 2 us hashing. Objects (trees, commits, graph chunks) are encoded by size alone, while content has been priced by its reads since E069. A graph chunk is decoded by every read of its graph, so pricing its encodings by that decode should store hot chunks as they are.
- Change: `encode_object` chooses a graph chunk's encoding by stored bytes + read weight x decode weight x canonical length:
  - canonical (stored as is): 0;
  - raw mode: 1/64;
  - zstd modes: 1.
  - Other objects stay chosen by size: the workload does not read them.
- Result (two runs; tests and clippy clean; every corpus exact with a clean fsck; every search exact): score 0.3221 / 0.3032 -> 0.2943 / 0.3077. Run 2 is not below E092's better run, so **not beyond the spread on both runs**.
  - repo-graph read 6.85 -> 3.27 ms, cost ratio 0.785 / 0.809 -> **0.399 / 0.399**. Stored bytes 50,013 -> 95,215: chunks stored as they are.
  - generated and transcripts change by a few hundred bytes (their function-library graphs).
  - The other corpora's movement in run 2 (synthetic-history 0.70, office 0.52) is on unchanged bytes.
- Verdict: **kept**. The effect on its target is far beyond noise; the score's spread hides it on one run.

### E094 chunk-trigrams (trigram postings per distinct chunk)
- Branch / parent: exp/094-chunk-trigrams / E093
- Hypothesis: transcripts' cost is 87% search, and its snapshots are cumulative. A word appears in every snapshot after its first use, so the per-file trigram index narrows to many files and each is scanned whole. E089's chunk index scans every distinct chunk. Combining them, trigram postings per distinct chunk would let a query scan only the few chunks holding all its trigrams, plus the seams, read from short stored contexts.
- Change (`src/search.rs`):
  - A third index kind (first byte `CHUNK_TRIGRAMS`): the chunk index, plus each trigram's chunk list, plus each seam's context (up to 31 bytes from each side).
  - A pattern of 3 to 32 bytes scans the candidate chunks (reading only their owners) and every seam context. Longer patterns use the chunk scan.
  - Exact: a match of up to 32 bytes lies inside one chunk, which then holds all its trigrams, or across one seam, inside that seam's context.
  - Planning chooses among trigrams, chunks and chunks with trigrams; both chunk kinds keep E089's 25% margin.
  - The chunk test now checks the new kind too, with patterns of 2–32 bytes straddling every seam.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact): score 0.2943 / 0.3077 -> **0.2659 / 0.2608**. Beyond the spread on both runs.

  | corpus | search CPU per query | index bytes (stored total) | cost ratio |
  |---|---|---|---|
  | transcripts | 0.79 -> 0.47 / 0.40 ms | 3,420,300 -> 3,646,238 | 0.83 / 0.74 -> **0.51 / 0.43** |
  | office | 0.113 -> 0.039 / 0.040 ms | 4,082,673 -> 4,230,763 | 0.43 / 0.52 -> **0.35 / 0.33** |
  | generated | 0.087 -> 0.031 / 0.033 ms | 715,119 -> 786,932 | 0.22 / 0.24 -> **0.15 / 0.15** |
  | repo-history | 1.02 -> 0.93 / 0.86 ms | 6,117,398 -> 6,711,935 | 0.17 / 0.12 -> 0.16 / 0.14 |
  | multiformat | 0.065 -> 0.053 ms | 5,789,679 -> 6,459,599 | 0.12 / 0.15 -> 0.15 / 0.15 |

  The indexes are larger, and the planner chose them because, under the heavy workload, the searches they save cost more than the storage. `examples/cross.rs` sets no search workload and is unaffected.
- Verdict: **kept**.

### E095 ablate-quotes (registry rerun: E056 under the cost objective)
- Branch / parent: exp/095-ablate-quotes (not kept; a one-line ablation) / E094
- Question (registry, high-priority ablation): does E056's quote function still pay under the cost score? It was measured under bytes (transcripts 113,507 -> 71,507 B).
- Change tried: the bench no longer proposes quotes (`propose_quotes` not called).
- Result (two runs; every corpus exact with a clean fsck): score 0.2696 / 0.2846 against E094's 0.2659 / 0.2608, within noise. Only transcripts changed: stored 3,646,238 -> 3,601,823 B (smaller without quotes), cost ratio 0.512 / 0.427 -> 0.465 / 0.395 (slightly better without).
- What it shows: under the heavy workload the quote derivation no longer pays. The planner still takes it, so it slightly misprices the derivation's reads. It still pays under bytes (E056).
- Verdict: **reference**. E056's registry row now holds only under bytes; the quote function stays as a capability for cold data.

### E096 ablate-programs (registry rerun: E022/E023 under the cost objective)
- Branch / parent: exp/096-ablate-programs (not kept; a one-line ablation) / E094
- Question: do the generated corpus's programs (E022 interpreter, E023 compiled WASM) still pay under the cost score?
- Result (generated only, two runs each, against two runs without the ablation): cost ratio 0.171 / 0.155 -> **0.065 / 0.075** without programs. Under the heavy workload the contents are stored raw either way, so the programs survive only as ledger facts. They cost storage (their WASM modules, 94,401 B as "Y") and fsck CPU (117–130 ms, 30 fsck a month): fsck re-executes every derivation. Without them: fsck 0.6 ms, repack 0.9 -> 0.1 s.
- What it shows: under the cost objective, facts that are not used to rebuild anything are paid for mostly by fsck re-executing them. This led to E097.
- Verdict: **reference** (the programs stay: they are the graph's record of how content was made, and they pay under bytes).

### E097 fsck-load-bearing (fsck re-evaluates only derivations reads depend on)
- Branch / parent: exp/097-fsck-load-bearing / E094
- Hypothesis: fsck evaluates every derivation and compares the result with its output id. When the output is also stored, three things make that evaluation unnecessary for reads: reads use the stored copy first, the same fsck hashes that copy, and a read that does go through a derivation checks its output hash anyway. Re-evaluating such facts (E096: WASM programs, 117–130 ms per fsck) can be dropped from the routine check.
- Change:
  - `fsck` evaluates a derivation only when its output is not stored. Every record is still hash-checked, and every stored content verified.
  - New `fsck_deep` also evaluates every derivation. The CLI's `fsck` command runs it, as its documented purpose is checking after dependency upgrades.
- **Guarantee change, stated:** a fact that has gone stale (for example an interpreter upgrade changing a program's output) but is not used for reads is no longer caught by the routine fsck. It is caught by `fsck_deep`, and by any read or repack that depends on it, since both verify outputs.
- Result (two runs; tests, clippy and fmt clean; every corpus exact with a clean fsck; every search exact; **every stored size and deterministic field identical to E094**):
  - Score 0.2659 / 0.2608 -> 0.2710 / 0.2670, within noise.
  - generated fsck 155 -> 1.1 ms, cost ratio 0.153 / 0.145 -> **0.076 / 0.091**; transcripts fsck 32 -> 3.8 ms; multiformat fsck 33 -> 22 ms.
  - The score's movement comes from timing noise on unchanged work, much of it on the baseline side (repo-graph baseline read 8.5–11.9 ms across runs; structured baseline search 3.0–4.0 ms).
- Note: the bench's run-to-run spread (about ±0.02 on the score) now hides per-corpus effects of this size. A longer minimum timing per operation would narrow it, at the cost of bench time (a benchmark change, not made here).
- Verdict: **kept** (identical output; a large effect on generated; the guarantee change is above).

### E098 min-timing (benchmark change: fastest runs, ingest and repack repeated)
- Branch / parent: exp/098-min-timing / E097
- Problem: at score 0.26, the bench's spread (about ±0.02 between two runs) hid per-corpus effects of 10–50% (E093, E097). Much of it came from timings of identical work: repo-graph's baseline read varied 8.5–11.9 ms, structured's baseline search 3.0–4.0 ms. Ingest and repack, a large share of several corpora's cost, were measured once.
- Change (`examples/bench.rs`):
  - `measure` takes the fastest run instead of the median. On a shared machine noise only adds time.
  - Each corpus is ingested and repacked on `PIPELINE_RUNS` = 3 fresh stores, keeping each operation's fastest run. The stores are identical, since the planner is deterministic.
- Result (three runs each; every corpus exact with a clean fsck; every stored size identical):

  | | median timing (E097) | fastest run only | fastest + 3 pipelines |
  |---|---|---|---|
  | score | 0.2710 / 0.2670 | 0.2390 / 0.2473 / 0.2460 | **0.2426 / 0.2397 / 0.2444** (2.0%) |
  | md | 0.26–0.31 | 0.226–0.276 (22%) | 0.275–0.276 (0%) |
  | generated | | 0.044–0.055 (23%) | 0.046–0.048 (4%) |
  | structured | 0.13–0.20 | 0.147–0.169 (15%) | 0.138–0.142 (3%) |
  | transcripts | 0.43–0.52 | 0.384–0.468 (22%) | 0.392–0.416 (6%) |
  | multiformat | | 0.151–0.170 (13%) | 0.134–0.141 (5%) |
  | repo-graph | 0.31–0.43 | 0.402–0.408 (1%) | 0.400–0.408 (2%) |

  Still noisy: office (11%) and invoices (22%; its searches take 0.012 ms). The bench now takes 24 s, against 21 s.
- Scores before and after E098 are not comparable: fastest runs are lower than medians on both sides.
- Verdict: **kept** (benchmark change; reference score 0.24).

### E099 north-score (benchmark change: the one-graph goal becomes a gated second score)
- Branch / parent: exp/099-north-score / E098
- Problem: the north star (one graph across file types, shared facts stored once) was measured only by `examples/cross.rs`, which nothing required anyone to run. Under the heavy workload storage is a small share of cost, so the cost score barely sees compression or sharing across types: E095 and E096 found quotes and programs lose under cost but still save bytes, and E081's multiformat store was 5.8 MB of raw files and an index against the baseline's 1.1 MB. `cross` also ran at the heavy weight (48.8) when given no argument, though this README said 0.001.
- Change:
  - `examples/bench.rs`: a north score after the cost score, in the same command. Each north corpus is stored as one repository and as one repository per file family, planned at `ARCHIVE_READ_WEIGHT`, repacked, read back byte-exact and fsck-checked (the hard gates), against zstd 3 with a dictionary trained on the same files. `ARCHIVE_READ_WEIGHT` uses `READ_WEIGHT`'s formula (E069) with `ARCHIVE_READS` = 2 decodes a month (an archive workload: ingest once, repack weekly, fsck monthly, two read passes, no search), about 0.00097. One line per north corpus (`north/cases`, `north/multiformat`: bytes, `vs_dict`, per-family split, `sharing_kernel`, `sharing_baseline`, the one-store repack's cost), then `_north` (`north_score`, geometric means of sharing, repack CPU and peak memory totals). `bench -- north` runs only these. The trained-dictionary baseline is factored into `with_dict`, shared by both scores. `_summary` is no longer printed when a filter ran no hot corpus.
  - `examples/cross.rs` deleted; the bench does its job, and its default-weight bug goes with it.
  - `examples/registry.rs`: `CURRENT` is `cost` and `north`. `registry.tsv`: E078, E079, E082, E084, E085 and E091, judged with `cross` at 0.001, are retagged objective `north`.
  - Docs: a "North score" section and the keep rule in this README (keep when either score improves beyond its tolerance on both runs and the other holds: cost about 2%, north 0.5%), AGENTS.md, SKILL.md, the kernel README, the architecture plan.
- Result (this machine, slower than the one E098 was measured on; E098 rerun once here for comparison; every corpus exact with a clean fsck; every hot stored size identical to E098):

  | | E098 (rerun here) | E099 run 1 | E099 run 2 |
  |---|---|---|---|
  | cost score | 0.2511 | 0.2507 | 0.2390 |
  | north score | | **0.3879** | **0.3879** |
  | north/cases one graph / baseline | | 391,496 / 870,159 (0.4499) | identical |
  | north/multiformat one graph / baseline | | 376,398 / 1,125,583 (0.3344) | identical |
  | sharing kernel / baseline (geometric mean) | | 0.9914 / 1.0452 | identical |
  | bench wall time | 284 s | 356 s | 355 s |

  - The north lines are identical byte for byte across runs, apart from repack timings (one-store repack CPU 28.4 s cases, 27.1 s multiformat).
  - Sharing: one graph is 2.3% smaller than the per-type stores on multiformat, 0.6% larger on cases. The baseline's one store is 9.1% larger than its per-type stores on cases and 0.2% on multiformat: one dictionary over mixed types hurts it.
  - Against `cross 0.001` on E098: identical bytes for code, md, pdf, html; data 51,701 -> 50,685 and office 162,273 -> 160,713 per-type, multiformat one graph 377,488 -> 376,398, from the slightly lower weight (0.00097 against 0.001).
  - The north corpora add about 70 s to the bench here (+25%), mostly the five per-type and one one-store repacks per corpus at low weight.
- Verdict: **kept** (benchmark change; north reference 0.3879 on two corpora; the cost reference is unchanged).

### E100 tables-corpus (benchmark change: the same records as a table and as a document)
- Branch / parent: exp/100-tables-corpus / E099
- Problem: the north star names spreadsheets, but neither north corpus has the same facts as a table and as prose. Office files in `north/cases` are different documents from its csv-less data files.
- Change:
  - `fixtures/tables`: each of the 24 fixture cases' first workbook by path (24 workbooks, 121,074 B) as xlsx, csv (LibreOffice, first sheet, formulas evaluated), json (the csv as row objects), md (a heading and a pipe table), docx (pandoc) and pdf (LibreOffice): 144 files, 1,181,083 B, committed as generated with `render.py` and `README.md.txt`. One TOTAL row keeps LibreOffice's Err:522 (a circular reference in the source workbook).
  - `examples/bench.rs`: `north/tables`, and csv in the data family. The cost score does not get this corpus.
  - Experiments README: the corpus in the north score section.
- Result (two runs; every corpus exact with a clean fsck; tests pass; every hot stored size and the two older north lines identical to E099):
  - north/tables: one graph 134,415 B against the baseline's 644,479 (**0.2086**). Sharing: the kernel's one graph is 7.0% smaller than its per-type stores (144,512), the baseline's 4.9% smaller (677,326). By family (kernel / baseline): data 4,524 / 7,307, md 2,805 / 5,819, office 54,128 / 80,216, pdf 83,055 / 583,984.
  - North score on three corpora: **0.3154** both runs (sharing kernel 0.9706, baseline 1.0130).
  - Cost score 0.2378 / 0.2531, within the spread of E099's 0.2507 / 0.2390.
  - Bench wall time here 356 -> 378 s; the north corpora take 108 s of it.
- Scores before and after E100 are not compared: the north reference moves from two corpora to three.
- Verdict: **kept** (benchmark change; north reference 0.3154 on three corpora).

### E101 byte-accounting (measurement: where the one-graph bytes go)
- Branch / parent: exp/101-byte-accounting / E100 (main at 4e04d23)
- Question: the one graph is only 3% smaller than the per-type stores (E100). Is that because cross-type sharing is hard, or because shared facts are a small part of the bytes? Like E001 for the hot corpora, account for every stored byte of each north one-store.
- Change (`examples/bench.rs` only; no kernel change; reported, not scored):
  - Each north line gets `roles`: `{"role":[bytes,shared,mixed]}` over every object of the one-store, summing to its stored bytes (rounding aside).
  - Roles: facts (`text:md`, `text:data`, `text:html`, `text:code`, `office:sheet-cells` for worksheet and shared-strings XML, `office:docx-body` for `word/document.xml`, `pdf:text-stream` for content streams with BT/Tf/ET) and format (`pdf:font`, `pdf:cmap`, `pdf:structure`, `office:parts`, `office:zip`, `dictionary`, `functions`, `structure:*` by object kind).
  - Attribution: every file is walked through its derivations and Q/M contexts. A part takes the role its own bytes show (`sniff`: font magic, `begincmap`, Office XML roots, the zstd dictionary and function library by their refs), else its parent's, so slices inherit. A group's stored bytes are split over its members by each member's size compressed alone (zstd 19). Objects no walk reaches are sniffed alone.
  - `shared`: bytes of objects reached from files of two or more families, that is information the one graph stores once across types. `mixed`: bytes of groups whose members come from two or more families (different types compressed together).
- Result (two runs; every corpus exact with a clean fsck; every stored size and north line identical across runs and to E100; north 0.3154 both runs; cost 0.2527 / 0.2544, within E100's 0.2378 / 0.2531):

  | role | tables (134,415) | multiformat (376,398) | cases (391,496) |
  |---|---:|---:|---:|
  | facts: text (md, data, html, code) | 3,948 | 48,550 | 163,221 |
  | facts: office sheet cells, docx body | 7,645 | 27,876 | 40,375 |
  | facts: pdf text streams | 5,587 | **85,010** | 0 |
  | facts total | 17,180 (12.8%) | 161,436 (42.9%) | 203,596 (52.0%) |
  | dictionary | **29,587 (22.0%)** | 30,708 | 11,827 |
  | pdf fonts | 24,224 (18.0%) | 45,895 | 0 |
  | derivation records | 21,105 (15.7%) | 23,344 | **69,105 (17.7%)** |
  | pdf structure | 16,437 | **95,450 (25.4%)** | 18,092 |
  | pdf cmaps | 5,261 | 11,186 | 0 |
  | office parts and zip | 18,839 | 7,121 | 76,005 |
  | trees, commits | 1,784 | 1,258 | 12,870 |

  - **Exact sharing across types is zero on tables and multiformat.** No object is reached from two families. On cases 50,177 B are (md 35,637 and code 14,540: the same text as a markdown and a code file), plus the 11,827 B dictionary.
  - What sharing there is comes from **mixed groups**: 40% of tables' bytes (53,770) and 27% of multiformat's (102,703) sit in groups whose members span families. That is statistical context, not information stored once.
  - In cases the "pdf text" row is 0 because its PDFs' content streams are not recognized by the sniff (their text operators differ from LibreOffice's); they count as `pdf:structure`.
- What it shows:
  - On **tables**, facts are 13% of the one graph. The records in csv, json and md together take 3,948 B, plus 2,316 B of sheet cells. A perfect record-set derivation across csv, json, md and the sheet (the planned E102) could save at most about 1–3 KB there, against about 100 B per derivation record. That is 0.3–0.7% of the north score, at or under its 0.5% tolerance, before overhead.
  - On **multiformat**, the same 24 texts are stored four times: md 24,719, html 23,831, docx body 27,876 and **PDF text 85,010** B. The PDF text is the largest single fact role in any north corpus and shares nothing literal with the others (E077: 0% shingle overlap, glyph-coded). Decoding it is the largest cross-type lever measured (E103 in the plan).
  - Format overhead the baseline also pays dominates tables: the trained dictionary alone is 22% of its one graph, fonts 18%, derivation records 16%.
- Verdict: **kept** (measurement capability; both scores hold).

### E102 pdf-text-layer (a PDF page's content stream as the text it shows)
- Branch / parent: exp/102-pdf-text-layer / E101 (exp/101-byte-accounting)
- Hypothesis: E101 found PDF text streams the largest fact role of multiformat (85,010 B) and sharing nothing literal with the md, html and docx copies of the same texts, because they draw glyph codes of subset fonts (`<0102...>Tj`). Decoding them through each font's `/ToUnicode` CMap and storing the text as one content per PDF lets the existing compression contexts find the same words in the other formats.
- Change (kernel; `src/pdftext.rs`, `src/container.rs`):
  - `pdftext::plan` splits a content stream into a skeleton (the stream with each decodable hex string replaced by an escape, its font and its length in characters) and its text (UTF-8, one line per text operator). A string is replaced only if encoding its text back through the CMap (longest match, lowest code first) gives its codes exactly; anything else stays verbatim. `build` inverts it.
  - `pdftext::content_fonts` relates zlib members to PDF objects (pages' `/Contents`, `/Resources`, `/Font`, fonts' `/ToUnicode`); `plan_zlib_at` now reports each member's offset.
  - For a `%PDF`, `plan_zlib` replaces the decodable content-stream members by one skeleton member and one text member and a new manifest segment, `2 level skeleton-span text-span fonts cmap-members`. The whole plan is checked to rebuild the file before it is offered; ingest verifies it as before. No new function id and no new derivation record: older manifests read the same.
  - Tried first and dropped: a builtin `pdf-text/1` derivation per content stream. Ingest never chose it per stream (each alone stored plain was smaller), and forced as a fact it saved 16,610 B on multiformat but added 22 KB of derivation records (six 32-byte ids per stream).
- Result (two runs; every corpus exact with a clean fsck; tests, clippy and fmt clean; every hot stored size identical to E101):
  - north/multiformat: 376,398 -> **332,403** B (0.3344 -> **0.2953**). PDF text streams 85,010 -> 31,272 B (skeletons), with the text members counted under `pdf:structure` by E101's sniff (95,450 -> 104,722). md 24,719 -> 19,628 and html 23,831 -> 20,292 now compress against the PDF text. Derivation records 23,344 -> 32,839.
  - Sharing on multiformat: one graph 6.0% smaller than per-type stores (0.9396), from 2.3% (0.9768).
  - north/tables: 134,415 -> 135,551 B (0.2086 -> 0.2103, +0.8%). Its 24 PDFs have 5,994 B of text in all: the skeleton and text members and their manifest entries cost about as much as they save (pdf text streams -2,279 B, fonts and zip +4,778 B as groups re-formed).
  - north/cases: unchanged (391,496); its PDFs' content streams do not decode through this path.
  - North score **0.3154 -> 0.3035** both runs (-3.8%, tolerance 0.5%). Cost score 0.2504 / 0.2527, within E101's 0.2527 / 0.2544.
- Verdict: **kept** (north improves beyond tolerance on both runs, cost holds).
- Next: the text member's role in the accounting (a `pdf:text` role); a per-PDF choice between the layered and plain plans would recover tables' 0.8%.

### E104 short-derivation-ids (derivation records with abbreviated ids)
- Branch / parent: exp/104-short-derivation-ids / E102 (main at 05628e2)
- Hypothesis: derivation records are 16% of tables' one graph, 10% of multiformat's and 18% of cases' (E101), and nearly all of a record is 32-byte ids that do not compress (E103: 102 B for a one-argument record). Trees already store resolvable ids as 8-byte prefixes (`abbrev.rs`). Doing the same for records shrinks every one.
- Change (kernel; `src/abbrev.rs`, `src/dict.rs`, `src/repo.rs`, `src/func.rs`):
  - Short record form after the tag: the output whole; the function as a one-byte builtin index (`func::BUILTINS`, append-only), an 8-byte prefix (a stored WASM module) or whole; a varint count; each argument's kind with a short-id flag, its id as an 8-byte prefix when exactly one stored or derivable id has it, else whole; range offsets as varints. Flagged by the `ABBREVIATED` mode bit, as trees are; stored only when smaller.
  - Decoding tries the prefixes' candidates and accepts only the canonical encoding that hashes to the record's id, so a later prefix collision costs a retry, never exactness.
  - The ledger is built from the records, and "derivable" means "a record's output". So outputs stay whole and the ledger reads in two passes: every record's output first (`derivation_output`, no resolution needed), then each record expanded against stored ids and those outputs.
  - `tests/exec.rs`: the one-record size it asserts goes from 102 to 53 B.
- Result (four runs, every corpus exact with a clean fsck; tests, clippy and fmt clean):
  - north/tables 135,551 -> **122,678** B (0.2103 -> 0.1904); records 22,234 -> 9,361.
  - north/multiformat 332,403 -> **310,415** (0.2953 -> 0.2758); records 32,839 -> 10,851.
  - north/cases 391,496 -> **361,242** (0.4499 -> 0.4151); records 69,105 -> 38,851.
  - North score **0.3035 -> 0.2793** on every run (-8.0%, tolerance 0.5%). Sharing: kernel 0.9589 -> 0.9624 (records shrink in the per-type stores too).
  - Hot stored sizes identical to E102 or smaller (generated -125 B, transcripts -553 B); `proxy_score` 3.8200 -> 3.8199, `read_score` 5.7268 -> 5.7266, `bytes_score` 5.7623 -> 5.7621.
  - Cost score **0.2646 / 0.2630 / 0.2594 / 0.2589** against main's 0.2504 / 0.2527 / 0.2414 / 0.2474 in runs alternated with it: about 4.5% higher, beyond its 2% spread. The rise is in measured time only. Its largest part is office (median 0.337 -> 0.418, from ingest). Office ingest executes the same work as on main: 10,039 M instructions on main, 10,037 M with E104 under callgrind. Wall time over the whole bench is lower (19.7 -> 19.6 s).
- Verdict: **pending**. North improves beyond tolerance on every run. The cost score does not hold by the keep rule's letter, although every deterministic measure holds or improves and the slower runs execute the same instructions (most likely code layout). Kept on its branch until that call is made.
- Follow-up (Nov 7, Eric's call: "it should be an option"): the short form is now `Repo::short_records`, **off by default**; reading accepts both forms whatever the setting, and repack's re-plan keeps it. `bench --short-records` measures with it on; `tests/exec.rs` asserts 102 B by default and 53 B with it on.
  - Off (two runs): every corpus byte-identical to main (all hot sizes, north/tables 135,551, multiformat 332,403, cases 391,496), north 0.3035, cost score 0.2484 / 0.2608. The second off run, with unrelated downloads running beside it, moved the cost score by 5% with identical bytes: the same size as the rise attributed to E104.
  - On (one run): north 0.2793 and every size as in E104's runs; cost score 0.2646.
  - Verdict: **kept**, as an option (a capability added while both scores hold by default). The trade-off and the other settings are mapped in `options.md`.

### E105 realworld-corpora (real files, and a corpus interface)
- Branch / parent: exp/105-realworld-corpora / E104 option (exp/104-short-derivation-ids at 232c6d1)
- Question: every score so far is on fixtures and generated corpora. How does the kernel do on files nobody made for it, and can someone ingest, browse and search a corpus without writing code?
- Corpora (`packages/kernel/realworld/fetch.py`, PyPI releases pinned and sha256-checked; nothing downloaded is run): the test documents python-docx (47 docx), python-pptx (68 pptx) and pypdf (72 pdf) ship; spreadsheets and ODF files from pyexcel-xlsx, python-calamine and odfpy (41); vega_datasets' data (20 csv/json/tsv); eight requests wheels and eight click sdists (one snapshot per release). GitHub-hosted corpora were left out: this session may only read repositories attached to it.
- Measurement (`examples/realworld.rs`, read weight 0.001, short records off): one fresh store per corpus, one commit (or one per release), repack, every file read back byte for byte, fsck; against zstd 3 with a trained dictionary (the bench's baseline), zstd 19 per file, and solid zstd 19 (long window) and xz 9 over all distinct files (`tar | zstd`, `tar | xz`, with no random access).

| Corpus | Files | Distinct bytes | Kernel | zstd 3 + dict | zstd 19 solid | xz 9 solid | vs best | Repack |
|---|---|---|---|---|---|---|---|---|
| docx | 47 | 1,155,108 | **563,619** | 850,034 | 629,167 | 629,492 | 0.896 | 7.2 s |
| pptx | 68 | 7,698,823 | **2,930,435** | 3,999,469 | 3,125,139 | 3,126,372 | 0.938 | 18.4 s |
| pdf | 72 | 15,345,736 | **4,826,271** | 6,336,442 | 5,210,576 | 5,029,704 | 0.960 | 66.8 s |
| sheets | 41 | 514,981 | **243,778** | 377,618 | 350,338 | 351,652 | 0.696 | 8.2 s |
| tables | 20 | 865,188 | 134,229 | 265,065 | 147,537 | **126,524** | 1.061 | 1.9 s |
| requests wheels (8 releases) | 8 | 501,909 | **96,865** | 380,610 | 270,003 | 270,780 | 0.359 | 3.9 s |
| click sdists (8 releases) | 8 | 2,411,765 | 2,149,467 | 2,390,433 | **2,114,069** | 2,114,752 | 1.017 | 2.0 s |

  - Every corpus exact with a clean fsck.
  - The kernel is smaller than the smallest archiver on five of seven, while keeping per-file random access the solid archives lack. The zip-based formats gain most: wheels 0.36 of the best archiver (zip members expanded, then shared across releases), sheets 0.70.
  - It loses on click's sdists (1.017): they are tar.gz, and gzip members are not expanded (`plan_zlib` looks only for zlib headers), so eight releases of mostly identical source share nothing. Next: E106.
  - It loses to xz on vega's tables (1.061), mostly two large JSON/CSV files where xz's model beats zstd's.
  - Repack time grows with the PDF corpus (67 s for 15 MB); the planner tries many plans per file.
- Interface (`src/bin/ikam/`): the `ikam` CLI gains `ingest DIR` (commit, repack, fsck and a byte-for-byte check of every file, with what it cost), `ls REV[:PATH] [-r]`, `grep TEXT [REV]` (also inside the parts the kernel stores separately: a docx's XML, a PDF's text layer), `stats` (objects and bytes by kind and encoding), `REV:PATH` and hex-prefix arguments everywhere, `--short-records`, and `serve [PORT]`: a read-only local web page on 127.0.0.1 to browse commits and trees, read or download files, see how each is stored (bytes, or a verified derivation and its inputs), search, and see what the store holds. `realworld/cli.sh DIR` runs ingest, ls, grep, stats and a checkout compared with `diff -r`.
- Verdict: **reference** (measurement and capability; no storage change, so the bench is unchanged).

### E106 gzip-streams (gzip members expanded like zlib streams)
- Branch / parent: exp/106-gzip-streams / E105
- Hypothesis (from E105): eight click sdists (tar.gz) store at 1.017 of the best archiver because gzip members are not expanded: `plan_zlib` looks only for zlib headers, and a gzip member is the same raw deflate behind a different header and trailer. Python's tarfile writes gzip at zlib level 9, which `deflate` reproduces, so the tars inside should expand and share across releases like zip members do.
- Change (`src/container.rs`): `plan_zlib_at` also accepts a gzip member at any offset: magic 1f 8b, method 8, no reserved flags, the optional extra, name, comment and header-CRC fields skipped, a raw deflate stream that ends, and a trailer whose CRC-32 and size match the inflated bytes. Header and trailer stay literal segments, as a zlib stream's header and Adler-32 do, so the manifest format is unchanged. The inflate loop is shared (`inflate_raw`). Test `gzip_members_expand_and_rebuild`: two members back to back with names in their headers rebuild exactly; a member with a wrong CRC stays literal.
- Result:
  - Real-world (`examples/realworld.rs`): click sdists **2,149,467 -> 433,893 B**, 0.205 of the best archiver (solid zstd 19: 2,114,069); every other real-world corpus byte-identical to E105. Repack 2.0 -> 21.7 s (the eight 1.3 MB tars are now planned).
  - Bench (two runs): every corpus byte-identical to E104's option-off runs (no fixture holds gzip), north 0.3035, cost score 0.2607 / 0.2521 against 0.2484 / 0.2608 for the same bytes before.
- Verdict: **kept** (a capability added while both scores hold).

### E107 parallel-recompress (store-level encodings on all cores)
- Branch / parent: exp/107-parallel-recompress / E106
- Question (from E105): repacking the 15 MB real PDF corpus took 65 s. Where does it go?
- Profile (temporary timers, pdf corpus): reading content 0.5 s, dictionary training 4 s, the three re-plans (already parallel) 14 s, and `recompress` 51 s: `encode_cheapest` (zstd 19, brotli, raw) 33 s and related-content contexts 17 s, all on one core.
- Change (`src/repack.rs`): `cheapest_encodings` computes `encode_cheapest` for every plain content before the loop, on all cores, largest first, each worker taking the next. It is a pure function of the bytes and the read model, so results do not depend on scheduling; the loop still checks each content against its id and uses the precomputed encoding. Contexts stay sequential, because each choice depends on the ones before. `examples/realworld.rs` now also prints the store before the first repack and peak memory.
- Result:
  - Real-world: every corpus byte-identical; repack wall time pdf 64.4 -> **43.9 s**, pptx 17.9 -> 14.1 s, docx 7.4 -> 5.5 s, sheets 8.2 -> 6.4 s, tables 1.8 -> 0.8 s (4 cores).
  - Bench (two runs): every corpus byte-identical, north 0.3035, cost score 0.2533 / 0.2546 against E106's 0.2607 / 0.2521. The cost score counts CPU time on all threads, so it does not reward parallelism; it holds.
  - Also measured: ingest alone stores the PDF corpus in 5,635,667 B in 8 s (already below zstd 3 with a dictionary, 6,336,442); the repack takes it to 4,826,271. Peak memory is about 570 MiB for 15 MB of PDFs, the next real-world limit to look at.
- Verdict: **kept** (a wall-clock speedup; bytes identical, both scores hold).

### E108 scale-profile (four pandas releases: where the time goes)
- Branch / parent: exp/108-scale-profile / E107
- Question: does the kernel hold up on a larger real corpus, and what limits it?
- Corpus: `realworld/fetch.py --large` adds the pandas source archives 2.2.0-2.2.3 (four tar.gz of about 4.4 MB each, 23.6 MB of tar inside each).
- Result (`examples/realworld.rs`, read_weight default, 4 cores): 17,583,071 B -> **3,270,055 B**, 0.204 of the best archiver (xz 9 solid 16,001,344; the gzip'd files barely recompress as they are); exact, fsck clean. Ingest 70 s (store 4,744,677 B before repack), repack **417 s**, reading every file back 26 s, peak memory **842 MiB**.
- Profile (temporary timers, two releases, cumulative): reading contents 13.5 s, dictionaries 21.6 s, the three re-plans 111.9 s, recompress 180 s (of which `cheapest_encodings` 51 s), total 198 s.
  - The re-plans (90 s) re-ingest every content in fresh stores, so each one re-inflates and re-plans every tar member again.
  - `cheapest_encodings` is bounded by the single largest content: on one 23.6 MB tar, zstd 19 takes 9.8 s (3,150,723 B) and brotli q11 39 s (3,056,772 B, 3% smaller). Brotli cannot split one content across cores.
- Next candidates, none tried yet: plan each content once and share the result across the re-plans; on contents over a few MB, run brotli only when zstd's result is within reach of the cheapest plan, or in parallel with zstd; stream large contents instead of holding every copy in memory (the 842 MiB peak).
- Verdict: **reference** (measurement; no storage change).

### E109 repack-deflate-memo (each large deflate once per repack; brotli cap as an option)
- Branch / parent: exp/109-repack-deflate-memo / E108
- Question (from E108): the pandas repack took 7 minutes. Where does it go, and how much of it is repeated work?
- Profile (temporary timers, four pandas releases, 4 cores): the four 23.6 MB tars were deflated 164 times for 232 s of CPU (the level search when ingest plans a container, hint checks, read checks, fsck, plan costs, reading contents back). Brotli took 44 s on one tar and lost; Q context frames 30 s, all lost; dictionaries 21 s.
- Change:
  - `src/container.rs`: `DeflateMemo`, a per-repack cache of deflate output for inputs of 1 MiB or more, keyed by content id and level; concurrent callers of the same key wait on one computation. `repack` installs it for its own run and removes it after; outside a repack nothing changes.
  - `src/ingest.rs`: a content whose verified hint is already a deflate_pack is not planned as a container again (the hint is the plan).
  - `src/repo.rs`: a range argument is sliced in place from the rebuilt source instead of cloning the whole source for each range.
  - `src/repack.rs`: dictionary training computes each sample's size alone once, not once per candidate dictionary.
  - New option `slow_codec_max` (default off): contents larger than this skip brotli when repack picks their encoding (see options.md).
  - `examples/realworld.rs` prints the store's own peak memory per corpus (`store_peak_kib`, reset per corpus) and takes `--slow-codec-max`.
- Tried and dropped: building the context frames ahead on all cores saved about 4 s on the PDF corpus and raised its peak 551 -> 695 MiB; removed.
- Result:
  - Pandas (17.6 MB of archives): stored bytes identical (3,270,055 B), exact, fsck clean. Repack **430 -> 130 s**, ingest 74 -> 42 s, reading every file back 25 -> 11 s, store peak 865 -> **791 MiB**. With `slow_codec_max` at 4 MiB: repack **82 s**, same bytes, peak 849 MiB.
  - Real-world (7 corpora): every corpus byte-identical, exact, fsck clean. Repack click 23.4 -> 8.6 s, pdf 55.0 -> 50.3 s (43.9 s capped), others within a few percent; peaks within noise of main (pdf 572 -> 568 MiB).
  - Bench (two runs): all 14 corpora byte-identical, north 0.3035, cost score 0.2419 / 0.2507 against main's 0.2497 / 0.2554; north repack CPU 72.0 / 69.8 -> 66.5 / 65.9 s.
- Verdict: **kept** (speed with identical bytes; both scores hold; the brotli cap is an option, off by default).

### E110 live-fs (the store as a filesystem you change while it runs)
- Branch / parent: exp/110-live-fs / E109
- Question (from Eric): can a corpus be used like a filesystem, with files added, changed, moved and deleted as it runs, searched and asked about, without a full repack per change? What does an edit cost, and does every file still read back exactly?
- Change:
  - `src/worktree.rs` (kernel): `Repo::edit_tree(root, edits)` applies put, mkdir, remove, rename and copy to a tree and rewrites only the trees on the changed paths; untouched subtrees keep their ids. A property test checks it against a model of the tree (200 random edit sequences; refused edits change nothing; a batch equals the same edits one at a time).
  - `src/container.rs` (kernel): `expand(bytes)`, the parts ingest expands a zip or a PDF into, for reading a binary file's text. After a repack a docx or pptx is often a concat of ranges of other files, so its derivation's arguments are not its members; text read from them was binary noise (100 MB of "text" from a 28.6 MB corpus).
  - `ikam live` (`src/bin/ikam/{live,dav,http,text,ui,ask,json}.rs`): one process owns the store. Edits come from WebDAV (`/dav/`, mountable in Finder), from watched folders (`--watch DIR[:PREFIX]`, polled every 500 ms), from the web page and from `ikam write|rm|mv|cp|mkdir`. Each edit is `put_content` plus `edit_tree`; edits commit after `--commit-after` of quiet (1.5 s); `repack` stays a separate step. Caches are keyed by immutable ids (sizes, also on disk; tree entries; 256 MB of bytes; extracted text). Search is exact case-insensitive over every file's text in memory (docx/xlsx/pptx XML, PDF content streams and text layers, tar members); questions rank 12-line passages by BM25 and list them as sources; `--llm` sends the question and passages to Claude (`claude-opus-5-5`, server-side fallbacks on) for an answer that cites them.
  - `realworld/live_bench.py`: drives `ikam live` over HTTP (upload, a random edit sequence checked against a copy of the tree, search freshness, watch lag, restart, fsck).
- Result (live bench, all 8 real-world corpora with pandas, 268 files, 46.2 MB, 4 cores):
  - Upload over WebDAV, one file at a time: 72 s, p50 46 ms, p90 274 ms; pandas tars 11 s each. Live store 15.5 MB; repack makes it 12.5 MB (214 s): edits stored as they come cost about 24% more than a repack (17% without pandas: 10.8 against 9.19 MB, the same as `ingest`'s 9.18 MB).
  - 400 random edits: p50 6-35 ms (change of a large text file 35 ms), about 1.2 KB new bytes each (mostly the rewritten trees; a one-line change to a text file over 100 KB adds 1.4 KB p50). Every touched file read back exactly after each edit and the whole tree at 8 checkpoints: **0 mismatches**; after restart all files exact, fsck clean.
  - Search p50 10.6 ms over 46 MB (1.1 ms without pandas), and for every plain-text file it returned exactly the files containing the term after the edits (47 terms). Question retrieval 60 ms p50.
  - Watched folder: a change on disk readable over WebDAV after 499 ms p50 (520 max).
  - Restart (index every file's text): 24 s with pandas (decompressing four 23.6 MB tars), 3.2 s without.
  - WebDAV conformance (litmus): basic 16/16, copymove 13/13; props 20/30 (dead properties are not stored); locks 20/32 (locks are granted but not enforced: one user). A davfs2 mount read and wrote files; listing a directory through it failed in this container's FUSE (PROPFIND itself answered).
  - The LLM step was checked only against a local stand-in for the API (request shape, key kept off the command line); no key was available here.
  - Bench (main and two runs of the branch, same machine): all corpora byte-identical, exact, fsck clean; north 0.3035 on all three; cost score 0.2493 on main, 0.2516 / 0.2434 on the branch (within its 2% spread). Nothing in ingest or repack changed; `edit_tree` and `expand` are new entry points.
- Not done, next: an incremental repack (re-plan only content added since the last one: the drift above is all new content), a text index kept on disk (restart time), enforced WebDAV locks, a persistent store of dead properties if a client needs them.
- Verdict: **kept** as a capability (both scores hold), on the branch until Eric says to merge.

### E111 live-host (running `ikam live` as a service another machine reaches)
- Branch / parent: exp/111-live-host / E110
- Question (from Eric's hosting plan): what does `ikam live` need to run on his Mac as a launchd service, reached from his other devices through Tailscale Serve, as a safe test that leaves the Mac's own files alone?
- Change (CLI only; nothing in the kernel library):
  - `--allow-host NAME` (repeatable): requests naming NAME, with or without a port, pass the Host check as 127.0.0.1 and localhost do. The listener stays on 127.0.0.1; reaching it from elsewhere is the proxy's job.
  - `IKAM_AUTH=user:salt:hash` (from `ikam passwd USER`, password on stdin): every request, page, API and `/dav/` alike, needs that user and password over HTTP basic auth, else 401 with `WWW-Authenticate`. The hash is BLAKE3 (derive-key mode) over a 16-byte random salt and the password, compared in constant time.
  - SIGTERM and SIGINT set a flag; the commit thread then takes the lock (so no edit or repack is half done), commits pending edits and exits 0. Before, both signals killed the process at once, leaving edits from the last `--commit-after` window out of the ref.
  - Found on the way, fixed: on a store with nothing committed, a repack's gc removed the empty working tree (reachable from no ref), and the next edit failed with "object not found" (the live bench on an empty corpus folder hit it as a 500 on MKCOL). The repack now stores the empty tree again; a unit test fails without the fix.
- Result:
  - Unit tests: Host matching (allowed name, name:port, case, no suffix tricks, other ports refused), base64, password check (right user and password pass; wrong password, wrong user, other scheme, bad base64 and no header get 401; a fresh salt each time), the empty-store repack.
  - By hand against the binary: no or wrong password 401, right one 200, the allowed name 200 with or without :443, another Host 403, WebDAV PUT 201 with the password and 401 without; a PUT with `--commit-after 60000` then SIGTERM: exit 0, "stopped", the file in the ref after restart, fsck clean (same with SIGINT).
  - Live bench (7 corpora without pandas, 400 edits): 0 mismatches, restart exact, fsck clean, edit p50 17 ms; the bench's own SIGTERM now stops the server cleanly. On an empty corpus folder: 0 mismatches, fsck clean (failed before the fix).
  - Bench: all corpora exact, fsck clean; north 0.3035 (unchanged); cost score 0.2510 (E110's runs 0.2434 to 0.2516). Nothing in ingest or repack changed.
- Not done, next: a text index kept on disk (restart 24 s with pandas), a slow password hash if the settings file could ever be read by others.
- Verdict: **kept** as a capability (both scores hold), on the branch until Eric says to merge.

### E112 markup-text-layer (an office XML part as its tags and its words, apart)
- Branch / parent: exp/112-markup-text-layer / main at 9bfb18c (E111)
- Question (from the README's measured gap): one graph is only a few percent smaller than one store per file type. Can the words of office files share with the same words in other formats, as E102 did for PDF pages?
- Gap today (main, `bench north`): cases one graph 391,496 vs per-type 389,058 (sharing 1.0063, one graph 0.6% larger); multiformat 332,403 vs 353,786 (0.9396); tables 135,551 vs 145,332 (0.9327); north 0.3035, sharing 0.9589 (the README's "about 3%, 7% on tables" predates E102).
- Probe (Python zstd 19, solid): md plus docx `word/document.xml` 54,158 B on multiformat; with the XML's text runs moved out of the tags and compressed with the md, the tags apart, 35,703 (-34%). tables (csv, json, md plus office XML parts) 10,582 -> 8,217 (-22%).
- Change (kernel; `src/markup.rs`, `src/container.rs`, option `Repo::markup_layers`, off by default):
  - `markup::split`: the skeleton is the markup with a 0x00 byte where each text run was; the text is the runs, one per line. A run with a newline stays in the skeleton; input with a 0x00 byte is not split. `build` inverts it (unit tests and a proptest).
  - `container::plan_zip_layered`: zip members with at least 64 B of text and text at least 1/16 of the member go into one skeleton member and one text member per zip, as manifest segment `3 level skeleton:u32 start len text:u32 start len` (like E102's segment 2). Parts with little text (styles, rels) stay whole, so identical parts still dedup. The plan is checked to rebuild the zip before it is offered. Reading accepts it at any setting.
  - With the option on, ingest plans zips this way; repack's re-plans copy the option. `bench --markup-layers`, `realworld --markup-layers`.
- Result (two full bench runs each, alternated with main; every corpus exact with a clean fsck; tests, clippy and fmt clean):

  | corpus | main one graph | option on | change | sharing main -> on |
  |---|---:|---:|---:|---|
  | north/cases | 391,496 | 375,713 | **-4.0%** | 1.0063 -> 0.9513 |
  | north/multiformat | 332,403 | 337,022 | +1.4% | 0.9396 -> 0.9489 |
  | north/tables | 135,551 | 135,230 | -0.2% | 0.9327 -> 1.0227 |

  - North score **0.3035 -> 0.3005** (-1.0%, tolerance 0.5%), identical on both runs.
  - Cost score: main 0.2635 / 0.2502, option 0.2446 / 0.2487 (within spread). Every hot corpus stores the same bytes (under the heavy read weight office stays near raw), but the office corpus's own cost rises 0.334 / 0.349 -> 0.396 / 0.459: splitting and rebuilding parts costs repack CPU.
  - Off: byte-identical to main on every corpus.
  - Real-world office files (`realworld`, single-type corpora): docx 563,619 -> 563,138 (-0.1%), pptx 2,930,435 -> 2,955,381 (+0.9%), sheets 243,778 -> 245,556 (+0.7%). With no other format holding the same words, the split only adds members.
  - Byte roles: cases docx body -14.1 KB and sheet cells -13.9 KB, text:code -11.9 KB (groups re-formed), skeleton and text members charged to office:zip (+19.0 KB, E101's sniff knows neither). multiformat docx body -15.3 KB but office:zip +18.1 KB, html +4.2 KB, md +1.3 KB: the docx text is cut into runs (one per line, words split by formatting), and 20 KB skeletons fill groups three at a time.
- What it shows: words apart from tags help where the same facts sit in several types (cases: one graph now 4.9% smaller than per-type stores, was 0.6% larger), cost a little where a file type is alone, and lose on multiformat. A per-file choice between the layered and plain plans (priced after repack) would keep the cases gain without the multiformat and real-world losses; so would an E101 role for skeleton and text members.
- Verdict: **kept as an option** (north improves beyond tolerance, cost score holds, office repack CPU rises; Eric's rule: trade-offs are settings). On its branch until Eric says to merge.
- Also found: on a shallow clone the bench's `repo-history` corpus is empty and `grep_queries` divides by zero (main too). A full or deeper clone runs.

### E113 finder-dav (why Finder stays on "Connecting", and a log that shows it)
- Branch / parent: exp/113-finder-dav / main at baab468 (E112)
- Question (Eric, Nov 8): on his Mac, Finder's Connect to Server at the Tailscale Serve name hangs on "Connecting" after the password, though the page opens in a browser and curl's PROPFIND gets 207; the log shows only startup lines, so nothing says whether Finder's requests arrive.
- What macOS sends (Apple's webdavfs source, `mount.tproj/webdav_network.c` and `WebDAVPlugin/WebDAV_Mount.c`): the plugin probes the URL for a proxy, NetAuth asks for the password, then the mount sends OPTIONS (needs DAV class 1, or 2 for read-write) and PROPFIND Depth 0 for getlastmodified, getcontentlength, creationdate and resourcetype, and must find a collection. Then statfs asks for quota-available-bytes, quota-used-bytes, quota and quotaused, Finder looks up its metadata names (._., .DS_Store, .hidden, .metadata_never_index, Backups.backupdb, ...), lists with Depth 1, and writes AppleDouble files under LOCK. Basic auth is fine over https.
- Replay: a Go reverse proxy shaped like Tailscale Serve (TLS in front, Host kept, X-Forwarded headers) in front of `ikam live` with IKAM_AUTH and --allow-host, over HTTP/1.1 and HTTP/2, with the WebDAVFS User-Agent: OPTIONS without a password 401 with the challenge, with it 200 (DAV: 1, 2); PROPFIND Depth 0 on /dav and /dav/, with Content-Length or chunked bodies, 207 with a collection and quota; the metadata names 404; Depth 1 207; LOCK on a new ._ file 201; PUT with Expect: 100-continue 201; MKCOL, MOVE, DELETE, UNLOCK as expected; no error or panic in the log. Nothing on the server side reproduces the hang, so the next fact has to come from the Mac.
- Change (CLI only):
  - `--log-requests`: one line per request, `TIME METHOD PATH host=HOST [CLIENT] -> STATUS MSms`, with ", no password" when the request had no Authorization header. The query (which may hold a search) and the password are never printed; a unit test checks it.
  - Every line `ikam live` writes (startup, watch, commit failures, stop, connection errors) starts with the UTC time, so a log tail over ssh shows which run a line came from.
- Result: tests, clippy and fmt clean. Ingest, repack and the store are untouched (bench not rerun for that reason).
- Next: run it on the Mac with `--log-requests` and retry Finder. No lines means Finder's requests never reach ikam (look on the client: `mount_webdav -i URL DIR` from Terminal, and `log show --last 5m --predicate 'process CONTAINS "webdav"'`); lines that stop at one request point at that request.
- Verdict: **kept** as a capability (logging off by default); merged to main on Eric's yes (Nov 8).


### E114 public-test (a copy anyone can read, and a self-test that publishes its result)
- Branch / parent: exp/114-public-test / main at c3afdca (E113)
- Question (Eric, Nov 8): put a test copy of ikam on Tailscale Funnel so Claude can watch it as it develops, without opening his own files: anyone may read, writes need a password, a test runner on the Mac uses that password and publishes what it finds.
- Gaps found in an open `ikam live`: request bodies read whole with no limit, no cap on the store, a thread per connection with no cap, header lines of any length, and repack/fsck/commit callable by anyone who can reach it when no password is set.
- Change (CLI only; nothing changes without the new flags except the header caps):
  - `--public` (needs `IKAM_AUTH`): guests (no Authorization header) get GET, HEAD and OPTIONS anywhere and PROPFIND on `/dav`; every other method, and all API POSTs, get 401 with the challenge. A guest's `llm` parameter is dropped, so no Claude answer is spent for them. A wrong password is refused, reads included.
  - `--max-body BYTES`: 413 from the headers alone (before 100 Continue or any body), and mid-stream for chunked bodies.
  - `--max-store BYTES`: 507 for PUT, MKCOL, COPY, MOVE, LOCK and PROPPATCH once the store is at the cap or the body would pass it; reads and DELETE still work. Size is measured once from disk, then counted from what the repo writes (re-measured after repack), plus desktop metadata held in memory.
  - `--max-conns N`: 503 (Retry-After 5) past N open connections.
  - Always: a header line over 16 KB or more than 100 headers gets 431.
  - `--public` defaults: 25M upload, 1G store, 32 connections, 30 s idle.
  - `realworld/selftest.py`: stdlib-only check of a running copy, under `/selftest/`: guest reads, 8 kinds of guest write refused, wrong password refused, no Claude answer for guests, 14 files (text, CSV, a Word file) read back byte for byte, search finds each by a word only it holds, ask's first source, an edit moves a word, rename/copy/delete/folder rename, commit; timings per kind. Writes `/dav/results/latest.json` and `history.jsonl` (last 200) with the password; never writes the password.
- Result: unit tests (limits over a real socket, store cap, guest method table, byte sizes), clippy and fmt clean, 10 full test runs with no flake. Manual: guest writes 401, password writes 201, guest reads 200/207, unknown Host 403, 30 MB upload 413, store cap 507 after 20 x 100 KB. Self-test passes twice in a row (about 13 KB stored per run). Live bench (default flags, 8 corpora, 264 files, 400 edits): 0 mismatches, search 0 wrong, fsck clean, restart 3.5 s, watch lag p50 515 ms; ingest untouched.
- Mac setup (dry run here with stand-ins for launchctl and plutil): `/path/to/ikam-test` with its own store, a generated test password (mode 600), `com.example.ikam-test` on 127.0.0.1:8739 using the private copy's auto-updated binary, and `com.example.ikam-test-run` that restarts the test copy when `ikam-live/deployed` changes and runs the self-test (also every 6 hours). Funnel on 8443 only; the private copy keeps 443 on the tailnet.
- Verdict: **kept** as options (all off by default); merged to main on Eric's yes (Nov 8).

### E115 robots (let web readers read the public test copy)
- Branch / parent: exp/115-robots / main at 44b5730 (E114)
- Question (Nov 8): with Funnel on, Claude's web reader refuses https://example-host.example.ts.net:8443/ as "robots_disallowed", including /robots.txt itself. ikam answered /robots.txt with 404 (`no such page`), which RFC 9309 reads as "allow", but some readers treat a missing or failed robots.txt as "keep out".
- Change: `/robots.txt` (GET or HEAD, after the Host check, before the password check) answers `User-agent: *` with `Allow: /` on a `--public` copy and `Disallow: /` otherwise. Nothing else changes.
- Result: unit test; manual: public copy 200 `Allow: /`, results still 200, PUT on /robots.txt 401; private copy 200 `Disallow: /`, everything else still 401, unknown Host 403. Tests, clippy and fmt clean.
- Not settled here: if the reader looks for robots.txt on the default port (443, the private copy, not on Funnel), this does not help; that would need the test copy on its own name on 443.
- Verdict: **kept**; merged to main on Eric's yes (Nov 9).

### E116 live-gc (the test copy's store grows each run)
- Branch / parent: exp/116-gc / main at 30d2042 (E115)
- Question (Nov 9): the public test copy's store grew 12,353 -> 26,020 -> 39,785 -> 53,847 bytes over four self-test runs, though each run clears its test area first.
- Cause: every live edit stores a new working tree (`edit_tree` per change), and a file written and then replaced, renamed away or removed before the next commit is in no commit. Nothing deleted either: `gc` ran only inside `repack`. Reproduced locally: six runs grew the store 12.2 KB -> 44.6 KB; `ikam gc` on a copy then deleted 455 objects and left 17.9 KB. The rest is history: each run's test files stay in its commits (about 1.5 to 4 KB a run, including the rolling results/history.jsonl, which shares all but ~700 bytes between versions).
- Change: `Live::gc` (commits pending edits, runs `Repo::gc`, re-measures the store for `--max-store`, re-stores the empty working tree when nothing is committed, as repack does); `POST /api/gc` (a write: password and `X-Ikam` header; guests get 401); option `--gc-every N` runs it after every N commits, off by default. `realworld/selftest.py` adds check 11, "gc frees space and keeps every file", and a guest refusal for `POST /api/gc`.
- Result: unit tests (garbage freed by more than half and every committed version reads back; `--gc-every 2` shrinks the store on commits 2 and 4; gc of an empty store then edits). Eight self-test runs against a local `--public` copy: 11 of 11 checks each, gc deletes ~130 objects a run, the store grows about 2 KB a run (history) instead of ~5 KB. `ikam gc` takes 0.06 s on a 9 MB store of 1849 objects. Tests, clippy and fmt clean.
- Not settled here: history is never dropped, so the test copy still grows about 2 KB a run (about 3 MB a year at four runs a day, well under its 1 GB cap).
- Verdict: **kept**; merged to main on Eric's yes (Nov 9).

### E117 bench-shallow (the full bench crashes on a shallow clone)
- Branch / parent: exp/117-bench-shallow / main at 30d2042 (E115)
- Question (noted Nov 8): `cargo run --release --example bench` on a fresh `git clone --depth 1` panics with "attempt to calculate the remainder with a divisor of zero" in `grep_queries`, after the earlier corpora have run.
- Cause: four corpora are built from this repository's own history at pinned commits (repo-history at 6cd6921; repo-graph, structured and transcripts at 0ca74e4). A shallow clone lacks those commits, `git rev-list` fails quietly, and repo-history comes out with no files. A partly deepened clone is worse: the commits exist with only part of their history, so a corpus comes out short and the run scores a different corpus without saying so.
- Change: before any work, the bench counts each pinned commit's history (`git rev-list --count`, 9 and 38 commits) for the corpora selected, and if one is missing or short, prints which corpora, which commit and how much history it has, and `git fetch --unshallow`, then exits with status 2. Naming only other corpora still runs. `grep_queries` asserts it has files instead of dividing by zero.
- Result: depth 1: fails at once naming all four corpora; depth 150: "commit 6cd6921 is missing; transcripts (commit 0ca74e4 has 21 of 38 commits of history)"; `md pdf` alone runs; after `git fetch --unshallow`, repo-history runs as before (exact, fsck clean). Measurement unchanged. Tests, clippy and fmt clean.
- Verdict: **kept**; merged to main on Eric's yes (Nov 9).


### E118 db-log-view (an event log and a last-write-wins view in the kernel)
- Branch / parent: exp/db-log-view / main at 8d95f82 (E117)
- Question (Eric, Nov 9): step 1 of the database roadmap: an append-only log in the kernel with idempotent appends and one last-write-wins view, built from today's objects, with no format change. Plan: `docs/plans/2026-11-09-db-log-view.md`.
- Change (kernel only, new `src/db.rs`): events (key, upsert or remove with subtree, idempotency key) stored as content; the log is a `G` graph keyed by 20-digit sequence numbers; an `ids` view refuses a reused idempotency key with different bytes and skips an exact retry; a `kv` view folds by last write wins, each value a byte range of the event that wrote it. A commit holds them under `db/`, beside any files. New error `IdempotencyConflict`. Nothing else changes.
- Result: `tests/db.rs`, 12 laws: the April fold tests (last write wins, remove whatever its idempotency key, independent keys, subtree remove, environment qualifiers), writes to distinct keys commute, exact read-back of values (empty, binary, 70 KB) and events, strict decoding, retries change no root, rebuilt views equal maintained ones every 50 of 300 random appends, older commits read their own version after gc, repack and fsck. All kernel tests pass; clippy and fmt clean.
  - Bench: all 14 corpora exact, fsck clean; north 0.3035 (unchanged); cost score 0.2389 (E110's runs 0.2434 to 0.2516; an append-cost test ran alongside at the start). Nothing in ingest or repack changed.
  - Append cost (ignored test `append_cost`): 1,000 single appends 6.2 s (8.5 ms each at the end); batches of 100, 0.21 s; 10,000 in batches of 1,000, 1.9 s (202 ms for the last batch). Every append rebuilds the three graphs, so cost grows with the database.
- Not settled here: in-place prolly edits (so an append touches one path), whether a reused idempotency key with different bytes should be an error (April skipped it), and the ledger scan at open.
- Verdict: **kept**; merged to main on Eric's yes (Nov 9).

### E119 db-inplace (graph edits that touch only the chunks an edit falls in)
- Branch / parent: exp/db-inplace / exp/db-log-view at e8ee290 (E118)
- Question (Eric, Nov 9): every append rebuilt the whole view; build in-place edits, with each view's log position recorded.
- Change (kernel only; no format change): `graph_edit` re-cuts only the chunks edits fall in, until the content-defined cut falls back into step, level by level; `graph_nodes_in` (batch lookup), `graph_prefix`, `graph_last`; `db_append` uses them; `db_rebuild` is a separate full fold with `put_graph`; views record their log position (`ids.at`, `kv.at` files under `db/views`), and appends to views behind the log are refused. `put_graph` is refactored to share sizing and parent building, unchanged in output.
- Result: 4 new graph laws (in-place root equals `put_graph` on graphs of height 0 to 2, chained random batches, appends at both ends from empty, emptying; one edit in 150,000 nodes writes only its path; label checks) and 2 new db laws (in-place views equal a full rebuild on a multi-chunk view with subtree removes across chunks; positions committed, defaulted and enforced). All kernel tests pass; clippy and fmt clean.
  - Bench: all 14 corpora exact, fsck clean; north 0.3035 (unchanged); cost score 0.2426 (E118 0.2389).
  - Append cost (`append_cost`, release, nothing else running): 1,000 single appends 6.3 s -> 3.1 s (last 9.5 -> 3.0 ms); batches of 100, 0.25 -> 0.20 s; 10,000 in batches of 1,000, 2.1 -> 1.8 s; 10,000 single appends 48 s, last 4.7 ms (not run before: about 10 minutes by extrapolation). Objects and bytes written unchanged.
  - Most of the remaining cost is the slice search when a new chunk is stored. Measured with it off (read weight 1.0, not committed): 1,000 single appends 0.9 s and 7.9 MB stored instead of 3.3 MB.
- Not settled here: storing new chunks plain on the fast path and leaving slices to repack (an option to measure next); reading every chunk above the leaves per edit (fine below about a million keys); clash checks for label keys outside the rewritten chunks.
- Verdict: **kept**; merged to main on Eric's yes (Nov 9).

### E120 db-loose-ends (label clashes and path-only reads in in-place edits)
- Branch / parent: exp/db-inplace / E119 at 05035da
- Question (Eric, Nov 9): fix both loose ends from E119: label-key clashes outside the rewritten chunks, and every upper chunk read per edit.
- Change (kernel only; no format change): `Keys::{Unique, Any}` with `put_graph_with` and `graph_edit_with`; the db's `ids` and `kv` views use `Keys::Any` (no edge points into them), the log keeps `Keys::Unique`. `graph_edit` walks from the root to the chunks edits fall in and right only until the cut falls back into step, instead of loading every upper level; output unchanged.
- Result: a real 64-bit key collision (`ke32051949435c933`, `kb4a1e63c25ad68d4`, found in about 10 minutes on 4 cores) used in a graph test and a db test (the db test fails with the views on `Keys::Unique`); a read-count test (fails on E119's code). All 134 kernel tests pass; clippy and fmt clean.
  - Bench: all 14 corpora exact, fsck clean; north 0.3035 (unchanged); cost score 0.2434 (E119 0.2426, timing noise: no bench path calls graph_edit).
  - One edit: 1M keys 10 to 11 ms and about 310 reads -> 2.1 to 2.3 ms and 45 to 55; 10M keys 94 to 105 ms and about 2,700 reads -> 2.2 to 2.4 ms and 45 to 47 (one run 8.8 ms). Upper chunks read per edit: 131 and 1,321 -> 3.
  - Append cost unchanged at its sizes: 1,000 single appends 3.3 s (last 2.9 ms); 10,000, 48 s (last 5.7 ms).
- Not settled here: `graph_edit` with `Keys::Unique` still checks only the chunks it rewrites (a key index for the log is an option past about 100M events); fsck does not check label keys; storing new chunks plain on the fast path.
- Verdict: **kept**; merged to main on Eric's yes (Nov 9).


### E121 results-no-cache (an old copy of the self-test results kept showing)
- Branch / parent: exp/121-results-no-cache / main at a86be84
- Question (Eric, Nov 9): a fetcher kept showing the 02:07 self-test results after the 6:00 run had replaced them; the server sent ETag and Last-Modified but no Cache-Control. Add the header.
- Change (CLI only): GET and HEAD under `/dav/results/` (the self-test's `latest.json` and `history.jsonl`, and the folder listing) send `Cache-Control: no-cache`, 304s included, so readers revalidate each time; ETag and Last-Modified stay, so an unchanged copy costs a 304. Other files are unchanged.
- Result: a dav test (results revalidated on 200, 304, HEAD and the listing; `a.txt` and `resultsx.txt` without the header). All 135 kernel tests pass; clippy and fmt clean. No storage change, so the bench was not rerun.
- Verdict: **kept**; merged to main on Eric's yes (Nov 9).

### E123 repack-dict-corruption (a later ingest dropped a group's stored bytes)
- Branch / parent: exp/123-repack-dict-corruption / main at d14be74 (E121)
- Question (Eric, Nov 9): ingesting `packages/kernel/fixtures/multiformat`, `docs`, `packages/kernel/src` and then `tests/fixtures/cases` into one store, each under its own ref, failed the fourth ingest with "object ... does not match its id", and fsck then failed 149 objects; `tests/fixtures/cases` alone into a fresh store was clean. Suspected: repack's dictionary move.
- Finding: not the dictionary. Every encoding in the failed store named the one dictionary (`7a6e7089`, still stored), and the fourth ingest failed in about 20 s, inside `snapshot`, before its repack. The 149 failures were the 49 members (`S 39f9173e8bf45ce8 ...`, docx XML parts from the multiformat ref) of one group whose stored bytes (`K`, 7,636 B) were gone, plus the 100 derivation records (59 `d`, 41 `D`) that read those members. A delete trace showed `apply_candidate` (from `container_candidate` unpacking a docx of the cases) deleting it. Cause: ingest indexes every stored content, groups included, so `new_template_candidate` anti-unified a new docx part with the group, recorded the group as a `fill` and dropped its stored bytes. Members read the group's stored encoding, never a derivation of it, so they were lost. `apply_candidate` kept the dictionary and "Q"/"M" contexts but not groups. Groups exist only after a repack, which is why one ingest into a fresh store was clean.
- Change (kernel only; no format change): `apply_candidate` keeps a content's stored bytes while another stored encoding needs them (`Repo::is_needed`: any encoding whose `needs` names the id, as gc counts it: a dictionary, a group, or a "Q"/"M" context). It replaces the "Q"/"M"-only check (`is_related_base`, now used by tests only) and costs the same one store scan per drop.
- Result: new test `tests/groups.rs` `ingest_never_drops_a_group_its_members_read` (40 small notes committed and repacked into a group, then a file shaped like the group under another ref; fsck must be clean, the group stored, every file exact, before and after a second repack): fails on main (the group is dropped and fsck is not clean), passes with the fix, 0.3 s. All kernel tests pass. The four-ingest repro now passes: every ingest prints fsck_failures=0 mismatched=0 (the fourth: 1,438 files, 233.7 MB in, 229.9 MB stored, repack applied) and the final `fsck` prints ok. Ingest and repack plans are otherwise unchanged. clippy and fmt clean.
  - Bench (`cargo run --release --example bench -- --json`, other jobs running): all corpora exact, fsck clean; north 0.3035 (unchanged); cost score 0.2391 (E120 0.2434; timing noise under load).
- Not settled here: `new_template_candidate` still credits a group's stored bytes it will not drop, so it can choose a template over plain bytes on a false saving (correct, but larger until the next repack). Refusing groups there needs a check cheaper than a store scan per file.
- Verdict: **kept**, on its branch until Eric says to merge.

### E124 section-reads (what a read of part of a file costs, by file type)
- Branch / parent: exp/124-section-reads / main at d14be74 (E121)
- Question (Eric, Nov 9: the most efficient format for I/O, query and permission, "including for targeting specific sections for specific actions relevant to the goals prompted by the user"): when a caller wants one section of a file (a 4 KiB byte range, one part of an Office file, one Markdown section), how many store objects and bytes does ikam fetch, against reading the whole file?
- Change: harness `examples/access.rs` (a counting wrapper over the store; phases `anatomy`, `e124`, then E125 to E127 below), run on the bench corpus `tests/fixtures/cases` (1,438 files, 1,906 objects, 229 MB stored). One fix found by it: `read_range` read a stored leaf to learn that it was stored, then read it again to rebuild the range, so a 4 KiB range of a 1 MB png fetched 2.2 MB. It now asks `store.has` (`src/range.rs`). Tried and dropped: storing range lengths on container plans; docx and pptx are rebuilt whole as containers, so it saved no reads.
- Result (fetched bytes per byte the caller wanted, medians by type):
  - png (580 files, whole files are one object): whole 1.0; a 4 KiB range 546 before the fix, 273 after (the object is fetched once; ranges inside one object are not separately addressable).
  - docx 31 reads for the whole file and 31 for any 4 KiB range (2.9 and 26 bytes per byte); pptx 59 and 59 (7.1 and 50): any range rebuilds the container. But each Office part is its own content id: the largest part of each of 238 Office files read alone fetched 2.76 MB, against 18.9 MB to rebuild the 238 files.
  - Small text files (md, json, py, pdf, log) are members of group objects: 2 reads, and 8 to 140 fetched bytes per byte (md median 78). A Markdown section read alone (134 sections) fetched 1.945 MB against 1.961 MB for the files whole: sections save nothing while their file sits in a group.
  - jsonl (24 files) is chunked: 3 reads whole and for a range, 0.22 bytes per byte whole (compressed).
- Gates: bench exact and fsck clean on every corpus; north 0.3035 (unchanged); cost score 0.2422 against 0.2391 on E123, a timing difference (the bench ran beside the per-tenant ingests and the 10M build; no ingest, repack or whole-file read path changed).
- Verdict: **kept** (the `read_range` fix; the measurements feed the design doc); on its branch until Eric says to merge.

### E125 query-paths (point, path, range and neighbourhood queries)
- Branch / parent: exp/124-section-reads / E124
- Question: what do the queries a linked store answers cost in reads today, and which would an index pay for: a path lookup, a database get and range, and outgoing and incoming links?
- Change: none to the kernel; phase `e125` of `examples/access.rs`.
- Result:
  - Path lookup (1,438 paths, depth up to 5): walking the trees takes a median 2 reads (max 9). A separate path index (59 objects, 62 KB, 1.8 s to build) takes 4 (max 7). No path index: the tree is already the index.
  - Database (100,000 keys): a get reads 10 objects cold; with a cache of decoded graph chunks, 1,000 gets touch 2,400 distinct objects (2.0 MB), 2.4 reads a get instead of 9.7. A 100-key range reads 13 objects (13 KB).
  - Links: outgoing links are 1 read (2 us). Incoming links ("what points at this file") need a scan of every object: 113,210 reads, 260 MB, 340 ms. A reverse-link view (328,942 edges, 18,356 objects, 33.9 MB, 40 s to build here from scratch) answers the same query in 8 reads, 12.8 KB, about 1 ms. (The view indexed every object in the store at that point, including the 100,000-key database built just before; over the bench store alone it holds 6,030 edges, E129.)
- Verdict: **measurement**. Proposes a reverse-link view kept per write (option S11 in `docs/research/store-backends.md`) and a decoded-chunk cache for graph reads; no path index.

### E126 permission-checks (what a permission check costs, and what crosses tenants)
- Branch / parent: exp/124-section-reads / E124
- Question: with each top-level case folder treated as a tenant (24 tenants), what does a read-permission check cost, which stored objects does one tenant's read touch, and what does it cost to keep tenants in separate stores?
- Change: none to the kernel; phase `e126` of `examples/access.rs`, and a script that ingests each tenant into its own store.
- Result:
  - Check cost: verifying a MAC'd capability 77 ns; a set of the tenant's reachable ids (129 ids, 107 reads and 0.7 ms to build) 20 ns a check; proving a file sits under the tenant's root by its tree path 8.1 us a file (trees cached).
  - Touch: a whole tenant read touches 105 to 203 objects; 85 to 179 are in its own tree, 17 to 24 are storage-internal (group, dictionary and delta objects), 0 to 2 are another tenant's content (identical files). 29 internal objects serve 2 or more tenants, and one 18 KB group serves all 24. So handing out storage objects, or answering "already stored" to a client, would leak across tenants; checks must sit at the file layer, where the server decodes and returns only the tenant's members.
  - A scoped edit inside one tenant: 16 reads, 13 writes, 3.6 KB written, against a store of 127,656 objects.
  - Separate stores: each of the 26 top-level folders ingested and repacked into its own store takes 644.1 MB in all, against 229.2 MB for the one shared store: 2.81 times. Nearly all of it is whole files that several tenants hold identically (the 1 MB diagram pngs of the l, m and xl cases: a 74 MB tenant alone is 74 MB, ratio 0.998); groups, dictionaries and deltas are a few hundred KB of it. Every store was exact with a clean fsck.
- Verdict: **measurement**. Enforce permissions on paths and files at the server, never on storage objects; per-tenant stores only where hard isolation is required, at 2.8 times the bytes here; keeping groups, dictionaries and deltas inside a tenant would cost little, since whole-file dedup is what tenants share. Never tell a client that content was already stored.

### E127 goal-reads (what a goal-scoped working set costs)
- Branch / parent: exp/124-section-reads / E124
- Question: when a user's goal is a few words ("invoice", "churn", "Q3"), what does it cost to find the files and sections it needs, and to hand a model only those sections?
- Change: none to the kernel; phase `e127` of `examples/access.rs` (the existing trigram index over 550 text files, 2.0 MB; windows of the matched lines and their neighbours).
- Result:
  - Index: 269 KB, 237 ms to build; reading every text file instead fetches 7.3 MB in 1,147 reads.
  - Search fetches about the matched files: "invoice" 13 files, 195 KB fetched in 2.8 ms (the 13 files whole are 184 KB); "revenue" 66 files, 1.0 MB, 12.7 ms. "Q3" has only two characters, so the trigram index can't narrow it, and the search reads every file (7.3 MB, 68 ms).
  - Windows hand the model about 6 times fewer bytes ("invoice" 30 KB against 184 KB) but cost more fetches than the whole files without a cache (257 KB), because each window rebuilds its group again.
  - Capabilities for the result: one id per matched file (13 for "invoice").
- Verdict: **measurement**. Proposes a short-term index (or exact-token postings) for terms under three characters, and a per-request decoded cache so windows cost no more than their files.

### E128 read-session (reads in one request share what they decode)
- Branch / parent: exp/124-section-reads / E127
- Question: E124 and E127 found that each read rebuilds the group, dictionary or container it needs, so reading the 2 KiB windows of a goal fetched more than reading the matched files whole, and Office files refetched the same shared groups. Does a request-scoped cache of decoded objects fix that, and what does it hold?
- Change (kernel, no format change, nothing written): `Repo::session(limit)` returns a `ReadSession` whose `read_content` and `read_range` share one decode context, so an object decoded for one read serves the next; past `limit` decoded bytes it forgets everything after the read that crossed it. Results are checked exactly as single reads. Test `read_session_equals_plain_reads` (whole and range reads, before and after repack, limit 0 and unbounded); phase `e128` of `examples/access.rs`.
- Result (bench corpus `tests/fixtures/cases`, 64 MiB limit):
  - Reading all 550 text files (2.0 MB): plain 1,147 reads, 7.31 MB fetched, 43.9 ms; one session 850 reads, 5.30 MB, 33.4 ms, holding 2.0 MB.
  - Goal windows, fetched by plain reads / by one session / for the matched files whole: invoice 257 / 166 / 184 KB; churn 460 / 198 / 248; triage 93 / 66 / 66; safety 816 / 440 / 483; revenue 1,515 / 909 / 1,007; onboarding 207 / 146 / 158; Q3 542 / 385 / 462. In a session a goal's windows always cost no more than its files whole, and hand the model about 6 times fewer bytes.
  - 238 Office files: 18.9 MB fetched read one by one, the same with one session per file, 2.68 MB with one session for all (they share groups and dictionary blocks).
  - All kernel tests pass.
- Verdict: **kept** (an option for callers that read several things in one request, such as the server's goal or search endpoints; plain reads are unchanged); on its branch until Eric says to merge.

### E129 needed-index (dropping a superseded copy without reading the whole store)
- Branch / parent: exp/129-needed-index / exp/123-repack-dict-corruption (E123)
- Question: keeping a reverse-link view (E125) current after small writes read the whole store many times over. Why, and can it be avoided without a format change?
- Finding: trees and graph chunks are stored as content, so ingest may plan a new chunk against its previous version and drop the old stored copy. Before any drop, `apply_candidate` asks whether another stored encoding needs that copy (a group, a context, a dictionary; E123), and `is_needed` reads every stored object to answer. Measured with `examples/writes.rs` on the bench store (2,498 objects, 229 MB, in memory behind a counting wrapper): adding 66 edges to the view after a one-file commit read 38,993 objects (3.68 GB, 16 full scans).
- Change (kernel; no format change): `Repo::needed_index` (off by default; `--needed-index` on the CLI, passed through to `ikam live`) keeps the prefixes that stored encodings name in memory, built from one pass on first use and kept current by `Repo::write` and by every write and replace in grouping and repack; `reset_projections` drops it. A stale entry (from a deleted encoding) only keeps bytes. It assumes the handle is the store's only writer while open: an encoding another process writes later is not seen. Tests: the E123 group test runs with the option off and on; `needed_index_stores_the_same_objects` (three ingests and two repacks give byte-identical stores off and on).
- Result (view update after each write, off / on):
  - one Markdown file: 38,993 / 632 reads, 3.68 GB / 1.9 MB, 546 / 164 ms;
  - one file of each of 13 types: 96,038 / 2,057 reads, 7.82 GB / 5.2 MB, 1,247 / 353 ms (the write itself 240 MB / 9.8 MB fetched);
  - a new folder of 36 files: 38,720 / 2,442 reads, 2.53 GB / 5.5 MB, 700 / 352 ms.
  - Still O(store) on every fresh handle: the first content write builds the slice matcher's seed index from every stored object (2,966 reads, 238 MB, 1.5 to 1.8 s here). Persisting that index would be a format addition.
  - The four ingests that found E123 (multiformat, docs, kernel src, then the
    1,438-file cases set, into one store), run on a frozen copy of the input
    with `--needed-index` on and off: fsck ok and 0 mismatched files both ways,
    and the two stores hold byte-identical objects. Wall time 333.5 s on and
    339.5 s off; the cases ingest stores 229,892,773 bytes either way.
  - All kernel tests pass.
- Verdict: **kept** (an option; recommended on for `ikam live`, which is the store's one writer); on its branch until Eric says to merge.

### E130 cold-handle (what the first read, write and lookup on a fresh handle read)
- Branch / parent: exp/131-seed-scope / exp/129-needed-index (E129)
- Question: every CLI command, and every process that opens a hosted store, starts with a fresh handle. What does its first operation read, and what would a rebuildable cache save?
- Method: `examples/cold.rs` on the cases store (1,906 objects, 229 MB, `FsStore` on local disk, warm page cache), each phase on a new handle behind a counting wrapper. 410 of its 1,438 files are stored only as derivations.
- Result:
  - Open, resolving a ref and reading the root tree, or reading a file stored as bytes: 0 to 3 objects, at most 40 KB, under 1 ms.
  - The first read of a derived file, or any first `has_content`: 1,907 reads, 229 MB, one full scan, 55 to 59 ms. The derivation ledger is a projection built by reading every stored object. The second read on the same handle: 1 read, 14 KB, 0.4 ms.
  - The first content write: the ledger (2,384 reads, 240 MB, 89 ms in memory), then the matcher's seed index (4,194 reads, 265 MB, 2.0 s). The second write: 4 reads, 0.5 ms.
  - A ledger cache, estimated outside the kernel: the store's 420 derivation records, length-prefixed in one file, are 159 KB and load in 0.1 ms (one read). That is 1,400 times fewer bytes than the scan, and one request instead of 1,906 on a remote store. The seed index cannot be cached the same way: about one anchor per 64 bytes of content, so near the size of the store.
  - Not measured: a cold disk on the Mac, and R2 latency per request (1,906 GETs per cold open; at $0.36 per million that is $0.0007, but the time is one round trip per object unless they are batched).
- Verdict: **kept** (a measurement; no kernel change). A ledger cache file would be an on-disk addition, so it waits on Eric. Until then, long-lived handles (`ikam serve`, `ikam live`) pay the scan once per process.

### E131 seed-scope (bound where new content looks for repeats)
- Branch / parent: exp/131-seed-scope / exp/129-needed-index (E129)
- Question: the first write on a handle reads the whole store to build the matcher's seed index (E129, E130), and new content may be built from any stored bytes, other tenants' included (E126). Can the caller bound where it looks, and what does that cost in stored bytes?
- Change (kernel; no format change): `Repo::seed_scope` (`None` by default, the whole store, as before). `Some(roots)` builds the seed index only from those contents and what they are derived from (the derivation closure); content written through the handle joins the index either way. Tests (`tests/scope.rs`): a scoped index uses repeats only inside its scope, reads back exactly, fsck clean; an empty scope stores plain bytes.
- Method: `examples/seeds.rs` on the cases store copied into memory, a new handle per case and scope, the ledger built first so the numbers are the seed index and the writes alone. Scopes: the whole store; the files of the tree; the top-level folders written to; those plus the folders the edits came from; empty.
- Result (reads, bytes fetched, ms, bytes written):
  - one Markdown file edited: whole store 2,980 reads, 238 MB, 1,697 ms, 137 B; folders written to 165 reads, 690 KB, 8.8 ms, 137 B.
  - one file of each of 13 types edited: whole store 3,344 reads, 247 MB, 1,627 ms, 16,176 B; files of the tree 2,749, 247 MB, 1,578 ms, 2,953 B; folders written to 758 reads, 85 MB, 541 ms, 2,914 B; empty 125 reads, 0.4 MB, 67 ms, 1,088,168 B.
  - a folder of 36 files copied to a new folder: whole store 4,143 reads, 243 MB, 1,639 ms, 21,817 B; folders written to (the new folder, so nothing) 556 reads, 0.8 MB, 104 ms, 13,096 B; folders written to and read from 1,229 reads, 4.1 MB, 120 ms, 8,478 B.
  - A narrower scope stored fewer bytes, not more, in two of three cases: the matcher keeps 4 candidates per seed, and on the whole store those slots fill with other copies before the best source. Only the empty scope lost (1.09 MB against 2.9 KB).
  - Isolation is partial: identical content is one object whatever the scope (content addressing), and container members and groups are found by id, not by seeds. With the empty scope 92 and 141 derivation arguments still named content from before the handle. So the scope bounds near-match reuse; permission checks still belong at the file layer (E126).
  - All kernel tests pass.
- Verdict: **kept** (an option; for `ikam live` and `serve`, scope a write to the folders it touches plus where its bytes came from, when the caller knows); on its branch until Eric says to merge.

### E132 short-search (one- and two-byte terms through the trigram index)
- Branch / parent: exp/132-short-search / exp/131-seed-scope (E131)
- Question: a goal of a few words often holds a two-letter term ("Q3", "AI", "EU"). E127 found the trigram index can't narrow those, so every search for one read every file. Can the existing index answer them, with no new structure?
- Change (kernel; no format change, no option): `Repo::search` with a trigram index and a pattern of one or two bytes takes the union of the posting lists of every trigram that contains the pattern, plus the contents in no posting list (those shorter than three bytes, which have no trigrams). Every occurrence of a short pattern in a content of three bytes or more lies inside one of its trigrams, so the candidates are exactly the contents that hold it; each is still decoded and scanned as before. Tests (`tests/search_short.rs`): results equal a scan for every one- and two-byte pattern over a small alphabet, with contents of 0 to 7 bytes, and for hand-picked cases. The chunk index (E089, E094) still scans every chunk for these patterns.
- Method: `examples/short.rs` on the cases store copied into memory, 526 text files (832 KB decoded, 7.1 MB fetched to scan them all, since most sit in groups); trigram index 393 KB, built in 1.9 s. Indexed numbers include loading the index on each query.
- Result (scan / indexed; fetched, ms):
  - "Q3" (26 files): 7.1 MB / 0.70 MB, 40.1 / 12.6 ms; "AI" (7): 7.1 / 0.34 MB, 44.5 / 9.0 ms; "EU" (8): 7.1 / 0.39 MB, 41.5 / 8.8 ms.
  - One byte: "%" (83 files) 7.1 / 1.4 MB, 38.3 / 15.7 ms; "x" (230 files) 7.1 / 3.3 MB, 40.3 / 29.5 ms.
  - Unchanged paths for comparison: "invoice" (13) 0.44 MB, 6.2 ms; "Q3 revenue" (0) 0.26 MB, 4.6 ms.
  - Results equal the scan's for every query. All kernel tests pass.
- Verdict: **kept** (no trade-off, so built in rather than an option); on its branch until Eric says to merge.

### E133 goal-pipeline (one goal end to end, defaults against the recommended options)
- Branch / parent: exp/133-goal-pipeline / exp/132-short-search, with exp/124-section-reads merged in for read sessions (E128)
- Question: E124 to E132 each measured one stage. What does a whole goal cost, from a few words to a committed edit with its view current, with today's defaults and with the options they recommend together?
- Method: `examples/goal.rs` on the cases store (2,277 objects after setup) copied into memory. Setup, not measured: a trigram index over its 550 text files and a reverse-link view over every object (E125). The goal: search a short term ("Q3"), hand a model the lines within two of each mention, change the term in five of the matched files, commit, update the view. Each configuration runs on a fresh handle, then a second goal ("AI") on the same handle. Defaults: no index at query time, plain reads, the whole store as seed scope, `needed_index` off. Recommended: the index (E132 for the short term), one read session per goal (E128), `needed_index` on (E129), and `seed_scope` set per goal to the edited files' folders plus the view's chunks and the trees on the edited paths (E131, `set_seed_scope`).
- Change (kernel; no format change): two fixes to `seed_scope` that this run found. A tree or graph chunk named in a scope now stands for the content holding its encoding and that content's derivation closure: without that, view chunks could not repeat their previous versions and one view update stored 404 KB instead of 237 KB (whole store: 227 KB). And `Repo::set_seed_scope` changes the scope and drops the seed index, so one handle can serve goals in different folders. Tests: `set_seed_scope_rebuilds_the_index`, `a_scoped_tree_lets_its_next_version_repeat_it`.
- Result (whole goal: fetched, ms, bytes written; the first goal includes the fresh handle's ledger build, 230 MB and 33 ms, in both):
  - First goal ("Q3", 27 files matched, 16.7 KB to the model from 347 KB of files): defaults 9.22 GB, 2,896 ms, 241 KB written; recommended 554 MB, 871 ms, 251 KB.
  - Second goal on the same handle ("AI", 8 files, 11.9 KB to the model): defaults 932 MB, 344 ms, 79.5 KB; recommended 6.3 MB, 214 ms, 78.3 KB. That is 148 times fewer bytes fetched.
  - By stage, first goal, defaults / recommended: search 7.3 / 0.73 MB; read 462 / 385 KB; edit and commit 240 / 89 MB (1,684 / 532 ms); view update 8.75 GB in 38 scans / 234 MB in one (the needed index's first pass).
  - What the recommended first goal still reads is two one-pass projections a fresh handle builds, the ledger (230 MB) and the needed index (234 MB), plus the scoped seed index (89 MB). The ledger cache in E130 would remove the first; the other two are once per handle.
  - The scope's price: 4% more bytes written on the first goal (251 against 241 KB). Without the view's chunks in scope, view updates stored 1.7 to 4 times more (404 and 308 KB).
  - fsck clean after every configuration. All kernel tests pass. Fixed bench: the same stored bytes; kernel op times within 5% of E132's (the cost ratio moved from 0.2655 to 0.3168 because the baseline's own ingest ran faster, 35.4 against 28.1 ms).
- Verdict: **kept** (a measurement, and two fixes to the E131 option); on its branch until Eric says to merge.

### E134 one-pass (a fresh handle's projections from one read of the store)
- Branch / parent: exp/134-one-pass / exp/133-goal-pipeline (E133)
- Question: E133's recommended first goal still read the store twice: once to build the derivation ledger (any first lookup) and once more for the needed index (E129) on the first write that could drop a copy. Both read every stored encoding. Can one pass build both?
- Change (kernel; no format change): when `needed_index` is on and its set isn't built yet, the ledger's pass over every stored object also feeds the needed index, and installs it if nothing built it meanwhile. Writes after that keep it current as before (`note_needs`).
- Result (`examples/goal.rs`, same setup as E133, recommended configuration):
  - First goal on a fresh handle: 554 MB to 324 MB fetched, 871 to 851 ms; the view update alone 234 MB in one scan to 3.9 MB in none. Bytes written the same (250,630).
  - Next goal on the same handle: unchanged (6.3 MB).
  - What the first goal still reads: the ledger's pass (230 MB) and the scoped seed index (89 MB).
  - Four-ingest safety check on frozen input, needed_index on and off: fsck_failures=0 and mismatched=0 on every ingest, fsck ok in both modes, objects identical.
  - All kernel tests pass. Fixed bench: every stored-bytes figure the same as E133's; timings within run-to-run noise (score 0.2433 to 0.2401).
- Verdict: **kept**; on its branch until Eric says to merge.

### E135 head-reads (the projection pass reads each object's header, not its bytes)
- Branch / parent: exp/135-head-reads / exp/134-one-pass (E134)
- Question: after E134 a fresh handle still fetches every stored byte once (229 MB on the cases store) to build the derivation ledger and the needed index. Both look only at an encoding's first bytes (its tag, mode, dictionary or group reference, or context ids), except derivation records, which are small. Can the pass fetch just those bytes?
- Change (kernel; no format change): `Store::read_head(id, n)`, the first n bytes of a stored encoding. The default reads the whole object and cuts it, so every store keeps working; `FsStore` reads only n bytes of the file and `MemStore` copies only n. The ledger's pass reads 256-byte heads; a derivation record shorter than that is already whole, and a longer one is read again in full.
- Result (`examples/heads.rs` on the cases store, 1,906 objects, page cache warm, two runs):
  - First lookup on a fresh handle: 229.2 MB to 192 KB fetched, 60 to 22 ms, the same 1,921 requests (no derivation record needed a second read). The same with `needed_index` on.
  - Every file's derivations match between whole and partial reads.
  - End to end (`examples/goal.rs`, as in E133 and E134): the recommended first goal fetches 94 MB, down from 324 MB (554 MB before E134, 9.22 GB with defaults); the same 250,630 bytes written. What it still reads is the scoped seed index (89 MB).
  - A store that can't read part of an object (the default) behaves exactly as before. On R2 a head is a Range GET: same request count, about 1,000 times fewer bytes.
  - The second goal's view update wrote 75,465 bytes here, 75,501 in E134 and 75,467 in E133: that varies from run to run because the scoped seed index adds content in hash-set order (E131). E136 looks at it.
  - Four-ingest safety check on frozen input: fsck_failures=0 and mismatched=0 on every ingest, fsck ok, objects identical with needed_index on and off and identical to E134's store.
  - All kernel tests pass, plus `tests/heads.rs` (heads are prefixes of reads on both stores; a reopened `FsStore` finds the same derivations as the handle that wrote them).
- Verdict: **kept**; on its branch until Eric says to merge.

### E136 seed-order (the same objects give the same encodings in every run and on every filesystem)
- Branch / parent: exp/136-seed-order / exp/135-head-reads (E135)
- Question: E133 to E135 each wrote a different number of bytes for the second goal's view update (75,467, 75,501 and 75,465) from the same store and the same edits. Where does the variation come from, and can it go?
- Cause: the matcher keeps at most four candidate sources per seed, the first ones added, so the order the seed index is built in picks among equal sources. The scoped index (E131, E133) added a tree's content closure from a hash set, whose order changes from run to run. The whole-store index and every other pass over `FsStore::ids` follow directory order, which on ext4 depends on a per-filesystem hash seed and on APFS is sorted, so the same writes could store different encodings on Eric's Mac than here. Content ids never change; only which equal encoding is stored.
- Change (kernel; no format change): `FsStore::ids` and `ids_with_prefix` return ids sorted, as `MemStore` already does, and the seed index sorts its input and each closure it adds.
- Result:
  - `examples/goal.rs` run twice: every stage writes the same bytes in both runs (second goal's view update 75,433 bytes in both).
  - Four-ingest safety check on frozen input: fsck_failures=0 and mismatched=0 on every ingest, fsck ok, objects identical with needed_index on and off and identical to E135's store (here, sorting the listings changed no encoding).
  - Fixed bench: every stored-bytes figure the same as E134's; timings within noise (score 0.2401 to 0.2472).
  - Sorting costs one sort of the id list per listing, well under a millisecond for the 1,906 ids of the cases store (estimated, not timed separately).
  - All kernel tests pass, plus `tests/order.rs`: file-store listings come in id order (fails without the change), and the same writes store the same bytes in a file store as in a memory store (a guard; it also passed before).
- Verdict: **kept**; on its branch until Eric says to merge.

### E137 file-scope (a goal's repeats looked for only in the files it touches)
- Branch / parent: exp/137-file-scope / exp/136-seed-order (E136)
- Question: after E135 the recommended first goal from E133 still fetches 94 MB, and 89 MB of it is the seed index for the edited files' whole top-level folders. Does a scope of only the files the goal edits (plus the trees and view chunks it replaces, as E133 found it needs) lose any repeats?
- Change: none in the kernel. `examples/goal.rs` gains a configuration, `recommended_scope_edited_files`, that sets `seed_scope` to the five edited files, the view's chunks and the trees.
- Result (cases store, same goals as E133):
  - First goal on a fresh handle: 94.0 MB / 797 ms to 7.7 MB / 318 ms; the edit-and-commit stage 88.7 MB / 506 ms to 2.5 MB / 28 ms. Bytes written 250,379 against 250,630: a little less, not more.
  - Next goal on the same handle: 6.3 MB to 5.7 MB; 78,086 bytes written against 78,298.
  - The edits here only change existing files, whose old versions are in the scope. A goal that writes new files from others it read (E131's folder copy) needs those sources in the scope too, so the rule is: what the goal reads and edits, not where it lives.
  - What the first goal now reads: search 0.73 MB, the model's windows 0.39 MB, edits 2.5 MB, view update 3.9 MB, the projections 0.25 MB.
  - No kernel change, so the safety check and the fixed bench are E136's.
- Verdict: **kept** as the recommended scope (in `options.md`); on its branch until Eric says to merge.

### E138 read-memo (an edit reads each old chunk once)
- Branch / parent: exp/138-view-update / exp/137-file-scope (E137)
- Question: after E137 the biggest part of a goal is the view update: 3.8 MB and 1,337 reads to add 214 reverse links on the first goal. Only 317 of those objects are distinct; 168 are read more than once, 993 reads are repeats. Where do they come from, and can they go without changing what is stored?
- Cause: each graph chunk (and tree) an edit writes is stored as content, so the matcher looks for repeats of it among the old chunks near it, reads each candidate, and reads it again to verify the derivation it proposes. The next new chunk does the same with mostly the same neighbours. A first try, sharing one rebuild context across the edit's chunk reads, removed only 13 of the 993: the repeats are on the write side.
- Change (kernel; no format change): `Repo::read_memo`, off by default (`--read-memo` on the command line). On, `graph_edit_with`, `edit_tree` and `put_content` keep the contents they read (up to 64 MB) until they return, and the matcher and every rebuild look there first. Only contents stored as bytes and checked against their id are kept, a kept copy whose stored object is dropped during the edit reads as it would without the memo, and evaluation work is counted as before, so plans are chosen exactly as without it.
- Result (`examples/goal.rs`, cases store, same goals as E133, new config `recommended_read_memo` = E137's recommended plus the memo):
  - First goal's view update: 3.85 MB / 1,337 reads to 1.01 MB / 446 reads; repeats 993 to 102. Edit and commit: 2.46 MB to 2.08 MB. The same 236,561 and 13,818 bytes written.
  - Next goal's view update: 3.16 MB to 0.80 MB; the same 75,221 bytes written.
  - Whole first goal on a fresh handle: 7.7 MB to 4.5 MB fetched; next goal 5.7 MB to 3.3 MB. Time is unchanged (within noise): this store is in memory, so the gain is in bytes and requests, which is what a file or R2 store pays for.
  - Four-ingest safety check on frozen input with `--needed-index --read-memo`: fsck_failures=0 and mismatched=0 on every ingest, fsck ok, objects identical to E136's store (needed index on, memo off).
  - Fixed bench (memo off, the default): every stored-bytes figure the same as E136's; score 0.2472 to 0.2399, timing noise.
  - `examples/goal.rs` also gains an `edit_reads` line (E139: what the first write of a goal reads) and `GOAL_FS=<dir>` to run on a file store.
  - All kernel tests pass, plus `tests/memo.rs`: a view and a tree edited with the memo on and off store identical objects with fewer reads on; nothing is kept once an edit returns.
- Verdict: **kept**; on its branch until Eric says to merge.

### E139 file-store (the recommended setup end to end on files and on packs)
- Branch / parent: exp/139-file-store / exp/138-view-update (E138), with exp/122-store-bakeoff merged in, so this one branch holds every experiment branch since E122.
- Question: E133 to E138 ran the goal on a store held in memory. What does the recommended setup cost on a real store, files or packs, and what is left?
- Change: `PackStore::read_head` reads only the first bytes of a record (E122's packs had only the default, a whole read cut short), with a test that heads are prefixes of reads from the write buffer, from the open pack and after a reopen. `examples/goal.rs` runs on a file store with `GOAL_FS=<dir>`, on packs with `GOAL_STORE=pack` too, and counts objects written per stage. Kernel defaults unchanged.
- Result (cases store, page cache warm, goals as in E133; recommended = E137's scope plus E138's memo):
  - First goal on a fresh handle, recommended: 4.5 MB fetched in all three; 735 ms on files, 351 ms on packs, 291 ms in memory. Defaults: 9.0 GB, 4.1 s on files, 3.4 s on packs.
  - Next goal: 3.3 MB; 421 ms on files, 186 ms on packs, 202 ms in memory.
  - On files the view update is most of the time (629 of 735 ms): it writes 425 objects for 214 links, and `FsStore` syncs each one. Packs sync twice per ref move whatever the count, so the same update takes 285 ms.
  - Same bytes written on every store (250,379 and 78,086); the fresh handle's lookup fetches 246 KB of heads on each.
  - Here each object written costs `FsStore` about 0.9 ms (create, sync, rename: 629 against 237 ms in memory for 425 objects). On macOS, Rust's `sync_all` is a full flush to the drive (`F_FULLFSYNC`), usually milliseconds, so the gap between files and packs should be wider on Eric's Mac: to measure there.
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E138; cost score 0.2435 against 0.2399, from timings and repack peak memory only.
  - All kernel tests pass, plus `tests/backends.rs` (pack heads).
- Verdict: **kept**; on its branch until Eric says to merge.

### E140 section-grants (what one goal may read and change, by byte range)
- Branch / parent: exp/140-section-grants / exp/139-file-store (E139)
- Question (Eric, Nov 9: permission "including for targeting specific sections for specific actions relevant to the goals prompted by the user"): E126 put permission checks at the file layer. Can a grant name sections of files instead, the lines a goal needs, for reading and for changing, and what does checking it cost?
- Change: `src/grant.rs`, a new API that stores nothing and changes no format. A `Grant` names, for one goal, file versions (path and content id) with read ranges and write ranges; `windows_around` builds the lines within two of each mention; `FileGrant::read` hands out only the granted ranges through range reads; `check_edit` finds what an edit changes in the old version (one region, aligned both ways for repeated bytes, else changed lines narrowed to their bytes) and allows it only when every region sits inside a write range and the old version is the granted one; `seal` and `verify` are a keyed BLAKE3 tag over a canonical encoding. Tests in `tests/grant.rs`; harness `examples/grants.rs` on the cases store, recommended setup, new handle.
- Result (goals as in E133; a file-level grant, E126, names the same paths whole):
  - Exposure: "Q3" matches 27 files (347 KB); its grant is 33 sections, 16.7 KB (4.8%). "AI": 8 files (79.6 KB), 13 sections, 11.9 KB (15%).
  - I/O: reading the granted sections fetches the same as reading the files whole (59 reads, 385 KB for Q3; 13 reads, 82 KB for AI; the new handle's first lookup, 192 KB of heads, apart). These text files are members of groups, which decode whole (E124), so sections cut what the caller sees, not what the store fetches.
  - Checks: a read check 14 to 22 ns; an edit check 14 to 20 us for the goal's edit (every mention replaced), 18 to 33 us to refuse an edit with a line outside the windows; another version or an ungranted file is refused in under 0.1 us. Sealing or verifying a grant 1.5 us (1.2 KB) to 3.2 us (3.7 KB).
  - What each grant allowed, over both goals (35 files): the goal's edit 35 of 35 under both; a line changed outside the windows, alone or with the goal's edit, 0 of 70 under the section grant against 70 of 70 under the file grant; an edit of a version the grant does not name 0 of 35 against 35 (a file grant names paths, not versions); a file not granted 0 under both.
  - All kernel tests pass.
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E139; cost score 0.2441 against 0.2435, from timings only.
- Verdict: **kept**; on its branch until Eric says to merge. A server option: grants per goal, checked at the file layer as E126 says; the format is unchanged.

### E141 replace-hints (a goal's scope names only its files; old chunks join the index as an edit reads them)
- Branch / parent: exp/141-replace-hints / exp/140-section-grants (E140)
- Question: with E137's scope and E138's memo, the first write of a goal still built a seed index over the edited files plus every tree and view chunk (1.78 MB in 1,125 reads of the 2.08 MB edit stage, E139). A new tree or chunk repeats the ones it replaces, and the edit reads exactly those. Can the scope name only the files, and the edit add what it reads?
- Change: `Repo::replace_hints` (off by default; a library option, since the CLI sets no scope). With a `seed_scope` set, `graph_edit_with` and `edit_tree` note each old tree or chunk content they read, and before each new tree or chunk is stored the noted contents join the seed index through the stored contents they are made of (as `ensure_index` indexes a scope). `examples/goal.rs` config `recommended_replace_hints`: scope of the edited files only, memo and hints on. Tests in `tests/hints.rs`: with hints a narrow scope stores what a whole-store index stores (4,205 bytes against 24,139 without hints), with fewer reads; without a scope hints change nothing.
- Result (cases store, goals as in E133, against E138's `recommended_read_memo`, first lookup's 246 KB of heads apart):
  - First goal: 4.21 MB in 1,757 reads to 2.50 MB in 625; the edit stage 2.08 MB to 0.48 MB (its first write 1.78 MB in 1,125 reads to 189 KB in 28); the view update 1.01 MB to 0.91 MB.
  - Next goal: 3.26 MB in 1,847 reads to 1.23 MB in 394 (edit 2.01 MB to 0.15 MB, view 0.80 MB to 0.63 MB).
  - Bytes written: 250,171 against 250,379 and 77,762 against 78,086 (the index holds fewer competing candidates); the same in two runs, so the order stays fixed (E136).
  - Time: about the same (on files 608 against 621 ms for the first goal, on packs 308 against 309 ms): what is left is the view update's syncs on files and its matching work.
  - All kernel tests pass.
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E140; cost score 0.2450 against 0.2441, from timings only (the bench does not set replace_hints).
- Verdict: **kept**; on its branch until Eric says to merge. Recommended with a scope; the format is unchanged.

### E142 session-search (a goal's search and its reads share one session)
- Branch / parent: exp/142-session-search / exp/141-replace-hints (E141)
- Question: after E141 the read stage still fetched what the search had just decoded (Q3: 59 reads, 385 KB), because `search` keeps no decodes between candidates and the read stage opens its own session. Can one session serve both?
- Change: `ReadSession::search` runs the search inside the session (`Repo::search_in`): the scan keeps decodes in the session's known set up to its limit instead of clearing them per candidate. `Repo::search` is unchanged (it calls `search_in` with a fresh context and clears per candidate, as before). `examples/goal.rs` config `recommended_shared_session`: E141's setup with one session for the search and the read stage, and a line for the search index load. Tests in `tests/session_search.rs`: the session's hits equal plain search's, reads of the hits then cost no store reads, and a 16-byte limit still finds the same hits.
- Result (cases store, goals as in E133, against E141's `recommended_replace_hints`, first lookup's 246 KB of heads apart):
  - First goal: 2.50 MB in 625 reads to 2.04 MB in 554; search 731 KB to 655 KB (repeats within the scan now hit the session); read 385 KB to nothing.
  - Next goal: 1.23 MB in 394 reads to 1.14 MB in 379; search 363 KB to 351 KB; read 82 KB to nothing.
  - Of each search, 269 KB in one read is loading the search index; a server that keeps it loaded across goals would bring the goals to about 1.77 MB and 0.87 MB.
  - Bytes written, edit and view update: the same.
  - What is left: the view update (906 KB and 632 KB, mostly view chunks it reads to place new edges) and the edit (477 KB and 154 KB).
  - All kernel tests pass (164).
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E141; cost score 0.2280 against 0.2450, from timings only (the bench ran beside goal runs; it does not search through a session).
- Verdict: **kept**; on its branch until Eric says to merge. A library API; nothing stored changes.

### E143 template-memo (a template candidate reuses the bytes the template search just read)
- Branch / parent: exp/143-template-memo / exp/142-session-search (E142)
- Question: E142's view update still read 102 objects twice in one edit (Q3), with `read_memo` on. Where from?
- Finding: a stack trace of every repeated read during the view update (183 over both goals) put all of them in `new_template_candidate`: `template_candidates` reads each similar chunk as a possible template through `read_content` (which keeps it in the memo), then `new_template_candidate` reads the same chunk's stored copy again to try it as the content a fill would replace, without asking the memo.
- Change: the memo also keeps the length of each kept content's stored encoding (all a template candidate needs besides the bytes, for its cost), and `new_template_candidate` takes both from the memo when they are there, and keeps what it reads otherwise. Off when `read_memo` is off; nothing stored changes. Test in `tests/template_memo.rs`: with the memo a put reads every object once, without it the template is read twice, and both store the same objects byte for byte. `examples/goal.rs` takes `IKAM_CONFIG=name` to run one configuration.
- Result (cases store, goals as in E133, E142's `recommended_shared_session`, first lookup's 246 KB of heads apart):
  - First goal: 2.04 MB in 554 reads to 1.59 MB in 441; the view update 906 KB to 522 KB (no object read twice), the edit 477 KB to 414 KB.
  - Next goal: 1.14 MB in 379 reads to 0.78 MB in 288; view update 632 KB to 327 KB, edit 154 KB to 102 KB.
  - Every configuration with the memo gains about as much (E141's 2.50 to 2.05 MB; E138's 4.21 to 3.76 MB); every configuration without it reads and writes exactly as before.
  - Bytes written: the same in every configuration (250,171 and 77,762 for the recommended setup).
  - What is left of the view update: first reads of the similar chunks a new chunk is compared with (154 reads, 476 KB for Q3).
  - All kernel tests pass (165).
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E142; cost score 0.2427 against 0.2280, from timings only (the bench does not set read_memo).
- Verdict: **kept**; on its branch until Eric says to merge. Part of `read_memo`; the format is unchanged.

### E144 object-templates (tree and view chunks planned only as slices)
- Branch / parent: exp/144-object-templates / exp/143-template-memo (E143)
- Question: after E143 the view update was still the largest stage, and it wrote 236 KB for a goal whose store grew by 76 KB. Where do the other bytes go?
- Finding: a new tree or view chunk is also tried as a fill of a template, and a template plan replaces the old similar chunk too (it stores the template, both fillers and two fills, and removes the old stored copy). Its cost counts only the net change, so it wins while writing three times what it saves, rewrites stored objects an edit did not change, and needs reads of the similar chunks to try.
- Change: `Repo::object_templates` (on by default, as before). Off, a stored tree or graph chunk is planned only as slices of earlier content (or stored plain); other content keeps template plans. `examples/goal.rs` config `recommended_no_object_templates` (E143's setup with it off) and a `net_growth` field on the view update (stored bytes added less bytes removed or replaced). Test in `tests/object_templates.rs`: four rounds of view and tree edits give the same roots with it on and off; off, no stored object is rewritten or removed, and fewer bytes are written.
- Result (cases store, goals as in E133, against E143's `recommended_shared_session`):
  - First goal: bytes written 250,171 to 85,734 (view update 236,353 to 71,916); the store grows by 71,916 against 75,944, so less space as well; objects written 425 to 308 in the view update; reads 1.59 MB to 1.48 MB.
  - Next goal: written 77,762 to 57,826; growth 54,961 against 57,971; reads 781 KB to 794 KB (2% more: one slice plan fetches a little more than the template plan it replaces).
  - Time, first goal: in memory 297 to 144 ms, on packs 332 to 159 ms, on one file per object 598 to 394 ms; next goal on packs 205 to 130 ms, on files 336 to 283 ms.
  - With it off, an edit only adds objects: nothing stored is rewritten or removed until a repack.
  - Configurations with it on read and write exactly as before. Not measured on the fixed bench's corpora, since the bench keeps the default.
  - All kernel tests pass (166).
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E143; cost score 0.2356 against 0.2427, from timings only (the bench keeps object_templates on).
- Verdict: **kept**; on its branch until Eric says to merge. Recommended off for a service that edits trees and views; the format is unchanged.

### E145 write-batch (a goal's writes share one memo, handed the files it read)
- Branch / parent: exp/145-write-batch / exp/144-object-templates (E144)
- Question: after E144 the edit stage still fetched 414 KB for five one-word edits (Q3): the first put builds the seed index by reading the edited files, each later put reads its old version again for its plan, and all five files had just been decoded by the goal's read session. Can a goal's writes share what was read?
- Change: with `read_memo` on, building the seed index keeps what it reads in the memo of the write in progress. `Repo::begin_batch` / `end_batch` run several writes (puts, a tree edit, a view update) as one edit, so what any of them reads is kept until the batch ends. `Repo::keep_read(id, bytes)` hands a batch bytes the caller already holds; they are kept only inside a batch, only if they hash to `id` and `id` is stored. `ReadSession::into_contents` gives back what a session decoded. `examples/goal.rs` config `recommended_batch`: E144's setup with the edit stage and view update in one batch, handed the read session's files. Tests in `tests/batch.rs`: a batch reads less and stores the same objects, handed bytes take each write down to one read (the stored length a template plan's cost needs), and wrong bytes or bytes outside a batch are ignored.
- Result (cases store, goals as in E133, against E144's `recommended_no_object_templates`, first lookup's 246 KB of heads apart):
  - The index-build memo alone: every configuration with the memo reads 80 KB less on the first goal and 18 KB less on the next (E144's setup 1.48 to 1.40 MB), with the same bytes written.
  - With the batch: first goal 1.48 MB in 391 reads to 1.13 MB in 338; the edit stage 414 KB to 70 KB. Next goal 794 KB in 288 reads to 744 KB in 277; edit 102 KB to 53 KB.
  - Time, first goal: in memory 144 to 136 ms, on packs 159 to 143 ms, on files 394 to 311 ms.
  - Bytes written and the store's growth: the same in every configuration.
  - Left in the edit: one read per edited file for its stored length (a store call for an object's length would remove it, an API addition) and the tree edit's own reads.
  - All kernel tests pass (168).
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E144; cost score 0.2443 against 0.2356, from timings only (the bench does not run batches).
- Verdict: **kept**; on its branch until Eric says to merge. Library API, used by a server's goal loop; nothing stored changes.

### E146 plain-object-chunks (tree and view chunks stored plain)
- Question: after E145 the view update reads about 405 KB on E133's first goal, mostly old view chunks and the sources that derived chunks would be built from. Would storing tree and view chunks plain (no slice or template plans) read less?
- Change: a temporary switch in `put_content` for chunks written by `write()` (tag t or g), measured on `recommended_batch` and taken out again; no code is kept.
- Result (first goal, view update): reads 405 KB to 393 KB (3% fewer); bytes written 72 KB to 391 KB; in memory 125 ms to 53 ms.
- Verdict: **discarded** (five times the writes and the store's growth for 3% fewer reads). Nothing on a branch; the row records the measurement.

### E147 stored-len (an object's stored length without reading it)
- Question: E145 left one read per edited file in the edit stage: a template plan's cost needs the old chunk's stored length, and bytes handed to a batch carry none. Can the store say how long an object is without returning it?
- Change: `Store::stored_len(id)`, a provided method that reads by default; MemStore answers from its map, FsStore from the file's metadata, PackStore from its index entry (a deleted entry is not found). A template plan whose old chunk's bytes are in the memo without a length asks the store; an error falls back to reading. `examples/goal.rs` forwards it through its counting store. `tests/backends.rs` checks the length on every backend, after a replace and after a delete; `tests/batch.rs` now asserts that a batch handed the files reads nothing in its writes.
- Result (cases store, `recommended_batch`, against E145):
  - First goal (Q3): edit stage 69.8 KB in 18 reads to 7.7 KB in 8; the goal 1.13 MB to 1.07 MB (1.31 MB with the first lookup's 246 KB of heads).
  - Next goal (AI): edit stage 53.2 KB in 15 reads to 2.3 KB in 6; the goal 744 KB to 693 KB.
  - Other configurations, the view update and bytes written: unchanged.
  - Time: within run-to-run spread. Five interleaved runs on packs, first goal: E145 143 to 186 ms (median 150), E147 143 to 170 ms (median 157). On files 338 to 372 ms against 381 to 429 ms over two and three runs, all of it in the view update, whose reads and writes did not change.
  - All kernel tests pass (168). The HTTP limits test failed once while the full suite ran beside a goal run, and passed alone three times and in the next full run.
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E145; cost score 0.2387 against 0.2443, from timings only (the bench does not run batches).
- Verdict: **kept**; on its branch until Eric says to merge. A store API addition with a default, so any backend works unchanged; nothing stored changes.

### E148 pending-view (a view's new edges kept pending, folded every few goals)
- Question: after E147 the view update is most of a goal: on E133's first goal it reads 405 KB (100 view chunks of about 3.6 KB and their path) to add 214 reverse edges, which land all over the view because the view is keyed by target id. Smaller view chunks first (measured with a temporary switch, not kept): 2-16 KB, 1-8 KB and 0.5-4 KB chunks read 374, 364 and 349 KB but write 81, 117 and 140 KB against 72 KB, grow the ledger's heads from 246 to 286, 351 and 456 KB, and take longer. Can the view be updated less often instead, log-structured?
- Change: `Repo::graph_prefix_pending` and `graph_node_pending`: lookups over a stored graph plus a map of pending edits (set or drop a label), giving what `graph_edit` of those edits would give, without writing it. Folding is the existing `graph_edit_with`. `examples/goal.rs` config `recommended_deferred_view`: E147's batch setup, with each goal's new reverse edges kept pending and folded once after the last goal; after each goal it looks up who links to each edited file's old version, through the pending edges. `tests/pending.rs`: over six rounds of random sets and drops, prefix and label lookups through pending edits equal lookups on the eagerly edited graph, and one fold gives the same root.
- Result (cases store, against `recommended_batch`):
  - View update per goal: first goal 405 KB in 260 reads to 22 KB in 26 (finding the new edges; nothing written), next goal 339 KB to 4.6 KB. The fold after both goals: 445 KB read, 96 KB written, against 744 KB and 127 KB for the two eager updates. The final view's root is the same.
  - The goals: first 1.07 MB to 684 KB, next 693 KB to 358 KB (search index load included, first lookup's heads apart).
  - Reverse lookups for the five edited files: the same 33 and 17 links; 51 KB and 50 KB read against 89 KB and 95 KB.
  - Time, medians of three runs: on packs the first goal 150 to 37 ms, the next 112 to 26 ms, the fold 159 ms; on files 368 to 48 ms, 290 to 35 ms, the fold 434 ms.
  - All kernel tests pass (169).
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E147; cost score 0.2407 against 0.2387, from timings only (the bench does not keep views pending).
- Verdict: **kept**; on its branch until Eric says to merge. Library API for a server, which keeps the pending edges in memory and folds every few goals or when idle. Pending edges are rebuilt from the objects written since the view's last fold, so nothing new has to be stored; recording which commit a view was folded at, so a restarted server knows where to start, would be a ref (an addition for Eric's yes).

### E149 exact-short-search (patterns of up to three bytes from the postings alone)
- Question: with the trigram index, E133's searches for "Q3" and "AI" still decode every candidate (655 KB and 351 KB with the index's 269 KB), though for patterns of one to three bytes a trigram's posting lists exactly the contents that hold it. Can such a search answer from the index alone?
- Change: `Repo::exact_short_search` (off by default): for patterns of one to three bytes the indexed candidates are the matches and are not decoded; contents too short to have trigrams and contents the index does not cover are still scanned. `tests/search_short.rs`: results equal a scan for every pattern of one to three bytes over a small alphabet (and a longer one), with an unindexed content in the set, and a search with the option on reads nothing but the index. `examples/goal.rs` config `recommended_exact_search`: E148's setup with the option on, and a measurement of what range reads of only the windows would fetch, positions taken from the decoded files.
- Result (cases store, against `recommended_deferred_view`):
  - Search: first goal 655 KB in 60 reads to 269 KB in 1 (the index), next goal 351 KB in 14 to 269 KB in 1; the same 27 and 8 files. 11.4 to 8.5 ms in memory.
  - The goal: unchanged (684 KB and 358 KB): the read stage now fetches the files the search no longer decoded, 385 KB and 82 KB.
  - Windows by range: the lines around each mention are 16.7 KB and 11.9 KB, but range reads of only those windows fetch the same 385 KB and 82 KB, because each file is one compressed object and a window is verified against the whole of it. A positional index alone would not help; windows need range-addressable stored files (E150).
  - All kernel tests pass (170).
  - The fixed bench on this branch: every byte count and the north score (0.3035) as on E148; cost score 0.2427 against 0.2407, from timings only (the bench does not set exact_short_search).
- Verdict: **kept**; on its branch until Eric says to merge. Off by default because results then rest on the index (a corrupt index would give wrong answers that a scan would not); on, a "which files mention" query costs only the index, which a server holds in memory.

### E150 framed-files (window reads if stored files were seekable)
- Question: E149 found that range reads of only the windows a goal needs fetch whole files: each file is one compressed object, verified against its id as a whole. What would window reads cost if each file were cut into independently compressed frames, each verifiable against the file's id with a BLAKE3 outboard (as Bao keeps), and what would the store grow by?
- Change: `examples/seekable.rs`, a measurement only; nothing is stored differently. Over the cases store's 550 text files: zstd level 3 of each whole file, of fixed frames of 1 to 64 KiB, a frame table and the hash tree's inner nodes; for the files holding "Q3" and "AI", the frames overlapping the windows (two lines around each mention) and their proofs. No dictionary anywhere, so the figures compare with each other, not with the store's encodings.
- Result:
  - Text storage, whole-file zstd 401 KB: 1 KiB frames 980 KB plus 122 KB outboard; 4 KiB 564 KB plus 29 KB (+48%, +55% with outboard); 8 KiB 481 KB plus 14 KB; 16 KiB 437 KB plus 7 KB (+11%); 64 KiB 403 KB.
  - Window reads, Q3 (27 files, 16.7 KB of windows, 106 KB whole-file zstd): 1 KiB frames 34 KB, 4 KiB 51 KB, 8 KiB 72 KB, 16 KiB 102 KB. AI (8 files, 11.9 KB of windows, 15 KB whole): 12.7, 12.9, 12.2 and 15.3 KB.
  - So frames halve window reads only at 4 KiB or less, for half again the text storage; at 16 KiB they cost 11% and save 4%. For reference, the store as it is fetched 385 KB for Q3's 27 files (347 KB raw), because each file is read out of a 64 KiB group (E151).
- Verdict: **measured**. Not worth a format addition for goals like these: windows are a large share of small files, and a goal that edits a file needs the whole file anyway. Worth it for large files read in sections (logs, long documents); there it would be a stored-format addition, for Eric's yes.
