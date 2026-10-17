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
