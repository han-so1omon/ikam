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
