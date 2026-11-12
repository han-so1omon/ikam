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
- Mac setup (dry run here with stand-ins for launchctl and plutil): a test-copy folder with its own store, a generated test password (mode 600), a LaunchAgent serving it on 127.0.0.1 using the private copy's auto-updated binary, and a second LaunchAgent that restarts the test copy when the private copy deploys a new build and runs the self-test (also every 6 hours). Funnel on 8443 only; the private copy keeps 443 on the tailnet.
- Verdict: **kept** as options (all off by default); merged to main on Eric's yes (Nov 8).

### E115 robots (let web readers read the public test copy)
- Branch / parent: exp/115-robots / main at 44b5730 (E114)
- Question (Nov 8): with Funnel on, Claude's web reader refuses the test copy's Funnel address (port 8443) as "robots_disallowed", including /robots.txt itself. ikam answered /robots.txt with 404 (`no such page`), which RFC 9309 reads as "allow", but some readers treat a missing or failed robots.txt as "keep out".
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

### E122 store-bakeoff (chunk and ref store backends, measured on ikam's own workloads)
- Branch / parent: exp/122-store-bakeoff / main at d14be74 (E121)
- Question (Eric, Nov 9): run a measured comparison of chunk and ref store backends, each behind the `Store` trait as an option: plain files, packs, SQLite WAL, LMDB, an LSM store (RocksDB, fjall), and any other strong fit (redb). Measure object count, open time (small, 1M, 10M keys), write and read throughput, single appends, batch commits, ref swaps, size on disk and crash safety. Research note: `docs/research/store-backends.md` (git packs and reftable, Dolt, dbt, ext4 and ALICE, Apple's fsync, LFS, LSM, WiscKey, Bitcask, BlobDB, ZFS/Btrfs/APFS, lakeFS, SQLite, LMDB, redb, fjall, R2, VFS for Git, overlayfs, File Provider). OpenResearch (`orx`) could not be installed here (proxy 403); the README's git loop was followed.
- Change (kernel; no format change to anything existing): `src/backends.rs` with a `Kv` trait and a `KvStore` adapter (objects buffered and committed atomically with the ref move, the CAS checked inside the transaction where the engine has one) over SQLite (WAL, `synchronous` FULL or NORMAL; `fullfsync` on macOS for FULL), LMDB (heed), redb, fjall (with and without key/value separation) and RocksDB (with and without blobs), each behind a cargo feature (`stores` for all); and `PackStore`: append-only packs of `id | len | bytes | crc32` records, sealed at 512 MB with a git-style fanout index, the open pack's index checkpointed every 64 MB and on close, every record's CRC checked on read, sealed lookups by interpolation search, refs as files. `FsStore` ref updates now lock with an OS lock instead of the lock file's existence. Harness `examples/stores.rs` (phases `corpus`, `db`, `build`, `probe`, `crash`); tests `tests/backends.rs` (one contract and a repository round trip on every backend, torn tails, checkpoints, stale ref locks) and a sealed-lookup unit test.
- Result (Linux VM, ext4, 4 cores; fsync here costs about 0.09 ms, so it is not the Mac's drive flush; other jobs ran alongside some phases):
  - The bench corpus `tests/fixtures/cases` (1,906 objects, 229 MB): write MB/s files 273, packs 769, SQLite 202, LMDB 172, redb 287, fjall 230 (330 separated), RocksDB 211; cold read MB/s files 827, packs 3,320, SQLite 1,052, LMDB 1,257, redb 557, fjall 462 (1,097 separated), RocksDB 609; open files 0.5 ms, SQLite 1 ms, LMDB 7 ms, fjall 39 ms, RocksDB 482 ms. redb took 338 MB for 229 MB.
  - Database appends (`db_append` + `db_commit`): one event per commit, mean (p99) ms: files 19.2 (108), packs 5.1 (12.6), SQLite 6.6, LMDB 5.9, redb 6.4, fjall 4.9, RocksDB 5.2. 10,000 events in batches of 1,000: files 21.3 s, packs 3.8, SQLite 10.5, LMDB 3.5, redb 6.0, fjall 4.8, RocksDB 5.3. Files changed per commit: files 25.5, packs 2, the engines 1. Bare ref swap p50: files 323 us, packs 361, SQLite 169 (11 with NORMAL), fjall 131. Flush calls per commit (strace; each is an F_FULLFSYNC on macOS): files 66, packs 2, SQLite 1.3 (0.3 NORMAL), LMDB 1.0, redb 1.0, fjall 1.2, RocksDB 1.1.
  - Scale, 64 to 511 byte objects: at 1M, size on disk (apparent / allocated): files 288 MB / 4.1 GB (one 4 KiB block per file), packs 372 MB, SQLite 394 MB, LMDB 1.03 GB, redb 1.08 GB / 751 MB, fjall 743 / 676 MB, RocksDB 590 MB; build files 348 s, packs 2.4 s, the engines 5 to 12 s. Cold read p50 at 1M: files 222 us, packs 39, SQLite 94, LMDB 115, redb 65, fjall 49, RocksDB 60; warm: LMDB 1.6, fjall 5.7, RocksDB 13.7, redb 32, packs 35, SQLite 46, files 49. At 10M (files skipped: over 40 GB of blocks): open packs 279 ms, SQLite 9, LMDB 15, redb 14, fjall 640, RocksDB 3,749; cold read p50 packs 190 us (351 before interpolation search), SQLite 155, LMDB 268, redb 85, fjall 130, RocksDB 142; warm LMDB 5.3, packs 37, redb 46, RocksDB 69, SQLite 92, fjall 104; size packs 3.7 GB, SQLite 3.9, LMDB 6.1, redb 6.5 / 5.3, fjall 3.5, RocksDB 4.3.
  - Packs first opened 1M objects in 44 s and 10M in 65 s: the open pack was rescanned with a seek per record. With the checkpoint and one sequential pass: 196 ms and 279 ms.
  - Crash (SIGKILL after 150 to 1,650 ms of a child that writes a 50 to 250 KB file, appends 1 to 40 events and commits, 25 kills per backend, twice): no acknowledged commit lost, fsck clean and views rebuilt byte-identical after every kill, on every backend (500 kills). The first runs also found that `FsStore` wedged a ref after a kill between taking `refs/<name>.lock` and the rename: every later update was refused (22 of 25 children in the second run). With the OS lock, 25 of 25 children commit.
  - All kernel tests pass (`cargo test --release`, and `--features stores --test backends`). The bench was not rerun: no ingest, repack or read path changed, and `FsStore`'s lock change affects only who may move a ref.
- Not measured here (needs the Mac): APFS and F_FULLFSYNC costs (66 drive flushes per commit on `FsStore` against 2 on packs is the number to price there), Finder over WebDAV, `clonefile`. Not built: a multi-pack index, a pack cleaner, per-pack Bloom filters for misses (a miss at 10M costs 130 us), an R2 backend (conditional PUT for refs, Range GET for packs).
- Verdict: **kept** (packs and the engines as options, `FsStore` stays the default; the `FsStore` lock is a fix); on its branch until Eric says to merge.

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

### E151 group-size (smaller groups for goal reads)
- Question: E150 found that the store fetches 385 KB to read Q3's 27 matched files (347 KB raw, 106 KB as whole-file zstd), because each small file is read out of a 64 KiB solid group and reading a member fetches its whole group. What do smaller groups cost and save?
- Change: `Repo::group_size` (64 KiB by default) bounds the groups repack packs small contents into; it is copied into repack's working handle and is part of the repack fingerprint, so a new size is planned again. The CLI takes `--group-size BYTES`. Measured first with a temporary switch, on copies of the cases store repacked at each size, then on the fixed bench at 16 KiB. `tests/groups.rs`: at 4 KiB no group a note reads is larger than 4 KiB (at 64 KiB one is), every note reads back, the store checks clean, an unchanged repack is skipped and a new size is planned again.
- Result:
  - Goal reads (E149's setup, read stage): 64 KiB groups 385 KB for Q3 and 82 KB for AI; 16 KiB 208 KB and 53 KB; 8 KiB 157 KB and 25 KB. At 4 KiB the repack found no plan smaller than the store's, so it kept the store as it was (repack applies a plan only when it saves bytes).
  - The cases store's bytes: 229,132,008 at 64 KiB, 229,131,301 at 16 KiB, 229,138,869 at 8 KiB.
  - The fixed bench at 16 KiB: north score 0.3035 to 0.3100, beyond its tolerance (stored bytes +0.1% on cases, +4.6% on multiformat, +1.8% on tables); cost score 0.2427 to 0.2341. As a default it fails the keep rule.
  - All kernel tests pass (171).
  - The fixed bench on this branch at the default size: every byte count and the north score (0.3035) as on E149; cost score 0.2406 against 0.2427, from timings only.
- Verdict: **kept** as an option, off by default (64 KiB); on its branch until Eric says to merge. A server whose work is goals reading a few files of a large store can trade about 2% of stored bytes for half the bytes per file read. Stores packed at any size read the same; nothing in the stored format changes.

### E152 held-index (the search index held across goals)
- Question: after E148 and E149 the next goal on a handle fetches 358 KB, and 269 KB of it is loading the search index again. What does a handle that keeps the decoded index between goals, as a server would, fetch?
- Change: none in the kernel. `examples/goal.rs` gains `recommended_held_index`: E149's setup, with the index from `search_index()` decoded once per handle and passed to every goal's search. `search` already takes the index from its caller, and contents not in the index are scanned, so the answers stay exact when files change between goals.
- Result (cases store, E133's two goals):
  - Next goal (AI): 358,351 to 89,224 bytes fetched (the index load drops from 269,127 to 0), 23.4 to 20.3 ms in memory; the same 8 matches, the same bytes written (2,865) and the same final view root as E149's setup.
  - First goal and the ledger pass unchanged (684 KB and 692 KB).
  - With 16 KiB groups (E151) on a repacked copy: next goal 329,093 to 59,966 bytes; first goal 506 KB.
  - Timed on stores (median of three, `GOAL_FS`): on packs the first goal takes 40 ms and the next 26 ms (16 KiB groups: 43 and 24 ms); on one file per object 59 and 39 ms (16 KiB groups: 84 and 52 ms, more objects to open). Here smaller groups fetch fewer bytes without being faster; they pay where bytes cost more than requests.
  - The fixed bench: every byte count and the north score (0.3035) as on E151; cost score 0.2311 against 0.2406, from timings only.
- Verdict: **measured**; nothing to merge beyond the example config. A server should load the search index once per handle and reuse it; then a goal after the first costs about 90 KB (60 KB with 16 KiB groups) against 932 MB with the defaults.

### Merge note: the rest of E122 (Nov 9, after E152)
- exp/139 had merged exp/122-store-bakeoff at its first commit, so E122's four later commits were in no branch after it: the bake-off harness (`examples/stores.rs`), pack checkpoints and CRC-checked reads, the FsStore OS ref lock, sealed pack lookups by interpolation search, and E122's own log, registry and options entries. They are merged into exp/152-held-index now. PackStore keeps E139's `read_head` and E147's `stored_len`; its whole read is E122's CRC-checked one.
- Gates: 174 kernel tests pass; clippy `-D warnings` clean with and without the store features; the fixed bench matches E152 in every byte count and the north score (0.3035); cost score 0.2451 against 0.2311, from timings (another job ran alongside). E133's goals fetch the same bytes (684 KB and 89 KB).
- `packages/kernel/scripts/mac-bakeoff.sh` runs the measurements that need macOS (flush cost per commit, kills, APFS small files, E152's goal on files and packs, clonefile) in its own folder, as the user, with no sudo and no service or port.

### E153 fragment-cuts (how to cut content into fragments)
- Branch / parent: exp/153-fragment-edits / E152 (main badc682)
- Question: Eric (Nov 9) asked for experiments on fragments as the Python version had them (research note `docs/research/fragments-and-ir.md`). Which cut makes a good fragment: one that is shared across files and survives edits?
- Change: none in the kernel. `examples/fragments.rs` cuts the 534 md and text files of the cases (1,035,302 bytes) whole, by line, by paragraph block (ending after a run of blank lines, cut at a line past 16 KiB), by CDC at 256, 1,024, 4,096 and 16,384 bytes, and by line-aligned CDC. Per cutter: distinct fragments and their bytes, zstd 3 over the distinct fragments end to end, a stored-size proxy (that zstd plus 32 bytes per distinct fragment and 4 per reference), size distribution and fan-out.
- Result:
  - Proxy bytes: whole 175,489 (dedup 1.11); lines 626K (dedup 1.95, but reference overhead); blocks 265,571 (dedup 1.24, median block 122 bytes); CDC 256 262K; CDC 1,024 197K; CDC 4,096 and 16,384 about as whole; line-aligned CDC like CDC.
  - zstd over each cutter's distinct fragments: about 157 KB for every cutter. Finer cuts do not help storage; the matcher and zstd already find what is shared.
  - Blocks: 805 shared blocks (125.7 KB), the largest fan-out 52. Natural fan-out is mostly the case generator's boilerplate (23 files).
- Verdict: **measured**. Paragraph blocks are the cut for edits (E154), not for storage.

### E154 edit-survival (new bytes per edit, by cutter)
- Branch / parent: exp/153-fragment-edits / E153
- Question: how many new bytes and references does a small edit cost under each cutter?
- Change: none in the kernel; part of `examples/fragments.rs`: four edits per md file (a word replaced in a middle line, a paragraph inserted, a line deleted, a note appended), measured as new fragment bytes and changed references per cutter, and as the bytes the kernel's `put_content` of the edited file writes.
- Result: whole files 740 to 860 new bytes per edit; lines 0 to 101 bytes but about 27 to 30 references; blocks 115 to 176 bytes, 1 to 2.3 new fragments and 5.6 to 7.6 references; CDC 256 201 to 367 bytes; CDC 1,024 345 to 549 bytes. The kernel's `put_content` of an edited file writes 139 to 194 bytes today.
- Verdict: **measured**. Blocks give the fewest new bytes with few references, so a block is the edit unit for E155.

### E155 fragment-renders (a file stored as a render over its blocks)
- Branch / parent: exp/153-fragment-edits / E154
- Question: can one edit to a shared fragment reach every file that renders it without re-planning those files?
- Change: `func.rs` adds the builtin `render/1`, which concatenates like `concat/1` but is a fact (not in `is_plan`), so repack keeps it; `range.rs` reads ranges of a render through its fragments. New `src/fragment.rs`: `blocks`, `put_fragmented` (fragments plus one render fact; a content of one fragment is stored as itself), `fragments_of`, `FragmentIndex` (which files render each fragment), `propagate`. `tests/fragment.rs`: blocks cover their input; after `propagate` every file that rendered the old fragment reads as before with it replaced, nothing else changes, the index follows the edit and fsck is clean; a render survives repack. `examples/propagate.rs` compares `scan-rewrite` (today: read every file, splice, `put_content`), `index-rewrite` (a side index finds the files) and `render`, on the 343 md files of the cases with natural fan-out and with a notice inserted into 1 to 256 copies.
- Result: stored at rest after repack 69,332 bytes as plain files, 169,623 as renders (+145%). Read pass 17.2 ms plain, 24 to 25 ms as renders. Edits: every check and fsck pass; render writes fewer bytes than index-rewrite at fan-out 64 and above (E157 has the numbers).
- Verdict: **kept** as an option on its branch, not a default: renders cost too much at rest (see E156, E157). `render/1` is a new builtin, a stored-format addition, so it waits on Eric's yes.

### E156 render-short (renders with short records)
- Branch / parent: exp/153-fragment-edits / E155
- Question: how much of E155's cost at rest is the records, and does E104's `short_records` take it back?
- Change: none beyond E155; `examples/propagate.rs` runs `render` with `short_records` on.
- Result: stored at rest 120,903 bytes (+74% over plain). Edit time at fan-out 256: 69.7 ms against index-rewrite's 117 ms.
- Verdict: **measured**; E157 goes further.

### E157 render-runs (only shared blocks stand alone)
- Branch / parent: exp/153-fragment-edits / E156
- Question: if only blocks that appear in two or more files stand alone and each run of other blocks is one fragment, is the cost at rest small enough?
- Change: `put_fragmented(bytes, Some(&shared))` merges runs of unshared blocks; `FragmentIndex.inner` records the blocks inside run fragments, so a block that later becomes shared is still found and its run is rewritten (`Part::Splice`). The law test covers runs.
- Result (343 md files; bytes written per edit, render-runs against index-rewrite):
  - At rest after repack: 85,851 bytes (+24% over plain); read pass 14.5 ms (plain 17.2 ms). The index holds 824 fragments and is built in 6.25 ms.
  - Natural fan-out 23: about 10.7 KB against 18.0 KB, and 8.2 KB against 13.6 KB.
  - Fan-out 64: 22.0 KB against 30.0 KB; about 23 ms against 28 to 37 ms.
  - Fan-out 256: 85.6 KB against 115.3 KB; 85 ms against 117 ms.
  - Small fan-out: the same (about 0.9 to 1.7 ms). scan-rewrite takes 7 to 117 ms (7 ms even at fan-out 1, against 0.9 ms with an index).
  - Trees and records dominate the bytes written; the synthetic `fan/i/` layout inflates the tree bytes.
- Verdict: **kept** as an option, off by default: a store whose work is edits that reach many files pays 24% at rest for 26 to 40% fewer bytes per edit and a faster edit at large fan-out. On its branch until Eric says to merge.

### E158 render-locality (does one paragraph change only its piece of each format?)
- Branch / parent: exp/153-fragment-edits / E157
- Question: an md file has html, docx and pdf renders (`fixtures/multiformat`). Is a block's render a piece of the whole render, so a fragment edit can be spliced into each format instead of re-rendering it?
- Change: `scripts/render_locality.py` (pandoc 3.1.3, LibreOffice). pandoc with `SOURCE_DATE_EPOCH=1767225600` and `--metadata pagetitle=<name>` reproduces the committed html byte for byte. Per text: whether each block's render alone appears in the whole render (html; docx `word/document.xml` body), and three edits per text (blocks at 1/4, 1/2, 3/4) spliced against a full re-render; one pdf re-render per text; bytes the kernel writes for each new render (`ikam put`).
- Result:
  - html: 571 of 576 blocks local; 72 of 72 edits splice byte-identical to the full re-render. Rendering the block alone takes a median 13.9 ms against 58 ms for the document.
  - docx: 26 of 576 blocks local and 0 of 72 edits exact. pandoc numbers bookmarks (`w:id`), lists (`numId`, in document order) and places `bookmarkEnd` across the whole document. With those ids normalised, 206 of 236 blocks (the first 10 texts) are local.
  - Median times: full docx 77 ms, block docx 44 ms, pdf through LibreOffice 1,049 ms.
  - Median bytes the kernel writes for a regenerated render: html 488, docx 1,504, pdf 2,517.
- Verdict: **measured**. html renders can be spliced per fragment; docx needs a renderer that renumbers document-wide ids (an option, not built); pdf is regenerated.

### E159 fragment-grants (goal-scoped edits under sealed grants)
- Branch / parent: exp/153-fragment-edits / E158
- Question: can a goal's sealed grant (E140) scope a propagated edit, so it touches only the ranges the goal may write, without diffing every file?
- Change: `fragment.rs` adds `propagate_granted`: it plans every file's parts first, takes each changed range from fragment lengths, checks the grant per file (no file: `NoFile`; another version: `Stale`; a range outside the write ranges: `Outside`) and writes nothing unless every file passes. `grant::inside` is `pub(crate)`. `tests/fragment.rs` adds `refused_grant_writes_nothing`. `examples/fragment_grants.rs` (the cases' md files plus a notice in n copies, render-runs): no grant, a full grant, the E140 diff check of the same edit, half the files granted, another block granted, and a stale version.
- Result (median of 5; n = 16, 64, 256):
  - No grant against a full grant: 5.5 / 5.5 ms, 20.3 / 22.9 ms, 88.7 / 81.9 ms (within noise), the same bytes written (5,766, 22,038, 85,616).
  - The diff check alone would add 1.0, 4.1 and 17.5 ms (it reads and diffs every file).
  - Refusals write nothing and leave the index as it was: half granted 0.6, 2.2, 9.2 ms; another block or a stale version 0.03 to 0.08 ms.
  - The sealed grant lists every file: 2,251, 9,087 and 35,772 bytes (about 140 bytes per file). A grant naming the fragment instead would be about 57 bytes whatever n is.
  - All kernel law tests on the branch pass.
- Verdict: **kept** with E155 (the check is free once edits go through fragments). A fragment-scoped grant is an option to build if grants that reach hundreds of files are common.

### E160 fragment-queries (search and insights over fragments, kept fresh)
- Branch / parent: exp/153-fragment-edits / E159
- Question: do indexes over fragments answer queries as exactly as file indexes, and can a propagated edit keep them fresh without a rebuild?
- Change: none in the kernel. `examples/fragment_query.rs`: the cases' md files plus a notice in 64 copies (407 files, 214,129 bytes), render-runs. 41 patterns (40 from the corpus by a fixed LCG, one notice phrase), each answer checked against a scan. A fragment search scans each distinct fragment and each distinct seam (63 bytes on each side of a boundary) once; a fragment trigram index narrows them. Per-fragment word counts, summed through each file's fragment list, checked against counting each file. Then the notice is revised by one `propagate` over 64 files.
- Result:
  - Search per query: scan 12.5 ms (178 KB decoded); stored index (E058/E094, 186 KB) 1.84 ms (36 KB decoded), built in 35 ms; fragment scan 0.45 ms (230 KB scanned, fragments held decoded: 159 KB plus 71 KB of seams); fragment trigram index (281 KB encoded) 0.11 ms, 10.8 KB scanned. All answers exact.
  - Word counts: per fragment 3.8 ms against 20.2 ms per file; summing all 407 files from fragments 4.2 ms, exact.
  - Goal reads: the paragraphs that hold a pattern are 8.0 KB per query against 29.2 KB for the files that hold it.
  - After the edit: the stored index left stale answers exactly but slower (5.2 ms, 66 KB decoded, since the 64 new files are scanned); rebuilding it takes 42 ms. The fragment indexes and counts update in 1.5 ms by reading one new fragment (101 bytes), and every answer and count stays exact.
- Verdict: **measured**. With files stored as renders, queries and insights per fragment stay fresh for the cost of the new fragment; the fragment trigram index is larger than the stored one (its seams and 33-byte keys), which a seam bound and a packed key list would cut. Nothing to merge beyond the example.

### E161 table-ir (one table, many formats)
- Branch / parent: exp/153-fragment-edits / E160
- Question: the Python version's `TableIR` (StructuredDataIR) with a deterministic renderer and a byte-identity drift spec: can a cell edit be made once and reach csv, json, md, docx and pdf (`fixtures/tables`, 24 workbooks)?
- Change: none in the kernel. `scripts/table_ir.py` lifts each csv into a TableIR: header, rows, per cell whether the spreadsheet typed it as a number (LibreOffice quotes only text cells), and two residuals (the md's title and source line; the csv's empty rows). Renderers follow `fixtures/tables/render.py`. A one-cell edit (the middle row's last numeric cell) is rendered to every format; the kernel's bytes written are the store files a put creates or rewrites. A scaled run (synthetic: one workbook's rows repeated to 10,000) shows the cost with size.
- Result:
  - Byte-exact renders: csv 24 of 24, json 24 of 24, md 24 of 24. Without cell types, 13 of 24 csv files reproduce; without the empty-row residual, 23 of 24.
  - Stored: the IR alone 6,515 bytes against 12,725 bytes for csv, json and md put as files (51%), with json and md rendered on read.
  - The edit: each rendered csv, json and md differs from the old in one contiguous range, 24 of 24. Bytes the kernel writes for all 24 tables: csv 3,105, json 3,566, md 4,124; docx 49,005 and pdf 92,905 (regenerated: docx median 45 ms, pdf 895 ms; not byte-identical to the committed files, which carry tool timestamps).
  - Scaled: rendering 10,000 rows takes 23 ms (csv), 99 ms (json), 11 ms (md); the kernel writes 12.7 to 16.1 KB per one-cell edit there, against 164 to 363 bytes at 100 and 1,000 rows.
- Verdict: **measured**. A typed table IR reproduces every committed text format and halves their storage; cell types are what the IR must keep. At 10,000 rows a one-cell edit costs the kernel 13 to 16 KB, so large tables want rows as fragments (an option, not built).
- Gates on the branch (E153 to E161): 178 kernel tests pass; clippy `-D warnings` clean on all targets; the fixed bench matches main (badc682) in every byte count, read cost and the north score (0.3035); cost score 0.2564 against 0.2392, from timings only (the bench never stores a render).

### E162 fragment-grant (a grant that names one fragment)
- Branch / parent: exp/162-fragment-options / E161
- Question: E159's sealed grant lists every file it reaches (about 140 bytes each). Can a grant name the fragment instead, at a constant size, with the same checks?
- Change: `grant.rs` adds `FragmentGrant { goal, fragment, within }` (its own seal tag, `seal`, `verify`, `allows`). `fragment.rs` adds `propagate_fragment_granted`: refused as `Stale` when the grant names another version of the fragment, as `NoFile` when a file that renders it lies outside `within`, otherwise `propagate_granted` with no per-file grant. `tests/fragment.rs` adds `fragment_grant_scopes_one_fragment`. `examples/fragment_grants.rs` adds three cases.
- Result (median of 5; n = 16, 64, 256):
  - Grant size: 85 bytes at every n, against 2,251, 9,087 and 35,772 bytes for the per-file grant.
  - Edit time: 5.7, 19.9 and 91.0 ms, the same as the per-file grant within noise.
  - Refusals (a file outside `within`, a stale fragment) take 0.002 ms and write nothing.
- Verdict: **kept** as an option beside the per-file grant. It is a sealed format addition (a new tag), so it waits on a merge decision.

### E163 docx-splice (a docx renderer that splices one block)
- Branch / parent: exp/162-fragment-options / E162
- Question: E158 found 0 of 72 docx splices exact, because pandoc numbers bookmarks and lists across the document. Can a renderer that maps those ids splice one edited block exactly?
- Change: none in the kernel. `scripts/docx_splice.py`: the old document.xml split into top-level items; the edited block's items (rendered alone) replace the old block's, list ids mapped to the document's, the context style (FirstParagraph or BodyText) taken from the position, and a heading's section bookmark renamed. The package is rewritten with the old entries' order, flags and deflate settings (Python's zipfile would change the flags).
- Result: 72 word edits and 24 heading edits. document.xml identical 96 of 96, every other part identical 96 of 96, the whole .docx identical 96 of 96. Median 35 ms per splice (mostly pandoc rendering the block) against 65 ms for the full render.
- Verdict: **kept** as a script. Edits that add or remove headings or lists were left to E169.

### E164 table-rows (rows as fragments for large tables)
- Branch / parent: exp/162-fragment-options / E163
- Question: E161 measured 13 to 16 KB written per one-cell edit at 10,000 rows. Do rows (or groups of rows) stored as fragments cut that?
- Change: `fragment.rs` adds `put_cut(bytes, cuts, fanout)`: a render over given pieces, optionally a tree of renders with at most `fanout` children. `examples/table_rows.rs`: synthetic tables (one workbook's rows with every digit redrawn, so rows do not repeat) as csv and md at 1,000, 10,000 and 100,000 rows; forms plain, rows, groups-64, tree-64, tree-groups-64; one appended digit in the middle row; everything checked byte-exact with fsck clean, before and after a repack.
- Result (csv; md is alike):
  - plain: 14 KB, 139 KB, 1.41 MB at rest; an edit adds 166 to 169 bytes at every size, in 2.7, 24 and 353 ms.
  - rows: about 10 times larger at rest; an edit adds 33 KB, 330 KB and 3.3 MB (the render names every row).
  - groups-64: +45% at rest; an edit adds 763 B, 5.4 KB and 51.8 KB, in 1.2, 7.9 and 60 ms.
  - tree-groups-64: an edit adds 763 B, 2.5 KB and 3.2 KB, in 1.1, 11 and 115 ms. tree-64 is slow (2.8 s per edit at 100,000 rows).
  - After a repack plain is smallest (100,000-row csv: 938 KB, against 1.18 MB for groups and 1.14 MB for tree-groups). Repack takes 36 s at 100,000 rows plain and 70 s for groups (the 100,000-row repack run was cut by the 30-minute limit; the rest is in the raw output).
  - Correction to E161: its 13 to 16 KB per edit at 10,000 rows was FsStore rewriting store files. In memory the kernel adds about 168 bytes for a plain put, at any size.
- Verdict: **discarded** as a default; plain puts already write about one row. Groups of rows are an option only for faster edits on very large tables (6x faster at 100,000 rows), at +45% stored.

### E165 table-ir-store (TableIR stored, json and md rendered on read)
- Branch / parent: exp/162-fragment-options / E164
- Question: store E161's typed TableIR (the csv itself, which carries the cell types, plus a residual sidecar) instead of csv, json and md, and render the other two on read?
- Change: none in the kernel. `scripts/table_ir_store.py`: four layouts ingested with `ikam ingest` (commit, repack, verify): the three text formats as files, the IR (csv plus `<name>.table.json`), and each with xlsx, docx and pdf added. json and md rendered from what `ikam cat` returns.
- Result:
  - Stored: 3,736 bytes against 6,519 for the three text formats (-43%); with the office formats 133,135 against 135,572 (-1.8%).
  - csv, json and md byte-exact 24 of 24; rendering json and md on read takes 0.14 ms for these tables.
  - Scaled (synthetic): rendering on read takes 241 ms at 10,000 rows and 2,656 ms at 100,000, against 6 to 11 ms and 35 to 94 ms to read the stored files.
- Verdict: **measured**. An archive option (cold tables, text formats only); for tables read often, storing the renders is faster and the kernel already stores them compactly.

### E166 render-range-lengths (bug: range reads rebuilt whole arguments)
- Branch / parent: exp/162-fragment-options / E165
- Question: E164's range reads of render trees cost far more decode work than the range. Why?
- Change: `range.rs` (`concat_range`) learned each whole argument's length by rebuilding it. It now asks `content_length`, which takes the length from the derivation for concat and render (summed recursively, memoized in the reconstruction context, with the existing stack guard), and rebuilds only otherwise. `tests/fragment.rs` adds `range_read_of_a_render_tree_skips_what_it_does_not_read`.
- Result: a 100-byte range of a 4,096-line render tree: decode work 99,064 to 400. The fixed bench is unchanged in every byte count, read cost and the north score (0.3035); the cost score moves with timings only.
- Verdict: **kept** (a fix to a default read path; no format change).

### E167 endpoints (one md edit carried to its html and docx)
- Branch / parent: exp/162-fragment-options / E166
- Question: one md paragraph edit, carried to the text's html and docx by splices (E158, E163) and stored in the kernel: what does it cost against re-rendering both, and should the html be stored as fragments?
- Change: none in the kernel. `scripts/endpoints_prep.py` makes 72 edits (24 texts, E158's three each), each html and docx splice checked byte-identical to a full pandoc re-render. `examples/endpoints.rs` stores the old md, html and docx, then the new ones, in two forms: every file with `put_content`, or the html as a render over its block pieces (`put_cut`).
- Result:
  - Rendering: the splices take a median 58 ms against 134 ms for re-rendering html and docx in full; 72 of 72 html and docx exact.
  - Bytes the kernel adds per edit (median): md 172, html 196, docx 1,951 (all files 2,340) as files; html 1,071 as fragments (all 3,210), because the new render names every piece.
  - Every file reads back exact and fsck is clean.
- Verdict: **measured**. For one file and its renders, splice the renders and store them as plain files: the matcher already writes about one paragraph for the html. Storing renders as fragments pays only when blocks are shared by many files (E157).

### E168 fragment-index-compact (a smaller fragment search index)
- Branch / parent: exp/162-fragment-options / E167
- Question: E160's fragment trigram index was 281 KB against the stored index's 186 KB, from its 33-byte keys and 63-byte seams. Can a packed form match it and keep answers exact?
- Change: none in the kernel. `examples/fragment_query.rs` adds a compact index: postings name fragments by position (the fragment list stored once, 32 bytes an id), seams are not stored (each file's fragment list gives them), and a seam is indexed only by the two trigrams that cross its boundary (any pattern of three bytes or more that crosses it holds one). A candidate seam is checked on the bytes around that boundary in each file that has it, walking into neighbouring fragments as far as the pattern reaches, so patterns of any length are exact (E160 was limited to 64 bytes).
- Result:
  - Size: 185,690 bytes (ids 21,600; fragment postings 162,408; seam postings 1,682) against 280,615 (-34%), about the stored index's 186,420. Bitmaps for dense lists save 98 bytes.
  - Query: 0.093 ms per query, 9.9 KB scanned, against 0.103 ms and 10.8 KB; all 41 answers exact, and 10 patterns of 200 bytes exact.
  - Build 7.8 ms against 20.8 ms; after the 64-file edit the index updates in 0.24 ms, every answer exact.
- Verdict: **kept** as the encoding for a fragment index (the example only; no kernel change).

### E169 docx-restructure (docx splices that add or remove blocks)
- Branch / parent: exp/162-fragment-options / E168
- Question: E163 spliced one block of the same shape. Can the docx renderer splice inserts and deletes, new headings and new lists, by renumbering what pandoc numbers across the document?
- Change: none in the kernel. `scripts/docx_restructure.py`: at ingest each md block's items are recorded (blocks markdown joins in the document, such as a list continued past a blank line, are kept as one group). An edit replaces a range of blocks with any md: bookmarks dropped, new items spliced in with fresh list ids, context styles set at both seams from what follows what in the old document, section bookmarks rebuilt from the headings (ids from 20 in the order their ends appear, names kept or made unique as pandoc does), lists renumbered from 1001 in order of first use, numbering.xml rebuilt to match. Where the new text touches a list, the neighbouring block is rendered with it, since markdown may join the lists. `docx_splice.rezip` takes several parts.
- Result (24 texts each): paragraph inserted 24/24, paragraph deleted 24/24, heading and paragraph inserted 24/24, two-item list inserted 24/24, heading deleted 24/24, one-word edit 24/24, and (synthetic, as no text has the case) a paragraph between two lists deleted so the lists join 24/24. The whole .docx byte-identical to a full re-render in all 168. Median ms, splice against full render: deletes 2.8 against 77; inserts 45 to 49 against 69 to 77; a list insert 95 against 78 (it renders its neighbour too).
- Verdict: **kept** as a script. Not covered: tables, footnotes, images, and edits to the title block.
- Gates on exp/162-fragment-options (E162 to E169): 180 kernel tests pass; clippy `-D warnings` and `cargo fmt --check` clean on all targets; the fixed bench matches exp/153 and main (badc682) in every byte count, read cost and the north score (0.3035).

### E180 formula-ir (a table IR with values, number formats and formulas)
- Branch / parent: exp/162-fragment-options / E169 (numbered from 180 because the storage thread holds E170 to E179)
- Question: E161's TableIR is lifted from the csv, so a formula's result is only text. Can an IR lifted from the workbook (the Python version's StructuredDataIR with ExpressionIR) render every committed format exactly and carry an input edit to every cell computed from it?
- Change: none in the kernel. `scripts/formula_ir.py`: the first sheet of each of the 24 workbooks as cells (value or formula, and number format). A formula is an OpShape (its AST with references relative to its cell) and an OpInstance (its cell). The renderer evaluates formulas (+ - * /, comparisons, IF, AND, SUM; a cycle shows as LibreOffice's Err:522), shows each value by its format (General, `$#,##0`, `0%`, `0.0%`; a General formula cell takes the format of the first cell it reads, as LibreOffice does), and writes csv with LibreOffice's quoting, then json and md (E161's renderers). Per formula, the first numeric input it reads is doubled (or set to 7 when 0); the truth is the workbook edited with openpyxl and recalculated and exported by LibreOffice, as `fixtures/tables/render.py` does.
- Result:
  - The unedited IR renders csv, json and md byte-exact for 24 of 24 workbooks.
  - 25 formulas in 9 workbooks are 12 shapes (`B2*12` in C2 and `B3*12` in C3 are one).
  - Input edits: 22 (3 formulas read no plain number: a SUM over formulas and two cyclic totals). csv, json and md exact 22 of 22; each edit changes 2.5 cells on average (the input and what is computed from it). The IR recomputes and renders in 0.12 ms against 872 ms for LibreOffice.
- Verdict: **measured**. Values, formats and formula shapes are enough to carry an input edit to every format exactly for this corpus; a formula evaluator and format renderer is the price, and the corpus exercises few functions.

### E181 fanout-endpoints (one paragraph edit to 64 files and their renders)
- Branch / parent: exp/162-fragment-options / E180
- Question: one paragraph shared by 64 md files is edited once. What does it cost to carry it to the 64 md files and their 64 html and 64 docx renders (192 endpoints), against re-rendering?
- Change: none in the kernel. `scripts/fanout_prep.py`: the 24 multiformat texts cycled to 64 copies, each with the same notice paragraph before its middle block (as E160), so 24 of the 64 are distinct. The notice is revised. Renders made by full pandoc runs, or by rendering the notice once and splicing it into each file (html as in E158, docx with E169's renderer). `examples/fanout_endpoints.rs` stores the 192 old files in one commit and the 192 new ones: every file with `put_content`, or the md files as render-runs (E157) with the edit as one `propagate`.
- Result:
  - Renders: 64 of 64 html and 64 of 64 docx spliced files byte-identical to full re-renders. Splicing all 128 takes 194 ms with 3 pandoc runs, against 8,477 ms and 128 runs to re-render (about 3.2 s if only the 24 distinct files were re-rendered).
  - Kernel (median of 3): all files as plain puts add 47,388 bytes in 158 ms; md as render-runs add 25,249 bytes in 222 ms (md 6,235, html 7,046, docx 11,968). At rest 228,047 and 230,669 bytes. Every file reads back exact; fsck clean.
- Verdict: **measured**. For an edit that reaches many files and their renders, rendering the changed block once and splicing it is about 44 times faster than re-rendering and exact; render-runs for the md halve the bytes the kernel writes. Duplicate copies in this corpus flatter the bytes; the render times are per file.
- Gates (E180, E181): scripts and examples only; clippy `-D warnings` and fmt clean; the kernel is as at E169's gates.

### E182 fragment-insights (summaries per fragment kept fresh through edits)
- Branch / parent: exp/162-fragment-options / E181
- Question: insights over a corpus (word counts and document frequency, dollar amounts, dates, headings, list items) are sums over files. If each fragment keeps its own summary, can an edit, including one propagated to many files, update the corpus totals by reading only the changed fragments, and how wrong do they get if left alone?
- Change: none in the kernel. `examples/fragment_insights.rs`: the multiformat md texts plus 64 `fan/` copies holding one shared notice (407 files, 214,129 bytes) stored as render-runs. A summary per fragment; a file's summary is the sum of its fragments'; corpus aggregates (word totals and document frequency, dollar total and count) take a file's summary out and the new one in. Stream of 22 edits: the notice revised by `propagate` (64 files), the most-shared block of the real texts revised by `propagate`, and 20 single-file edits. Policies: full recompute from every file; eager (fragments read only where the summary cache misses, aggregates updated per edit, checked equal to full after every edit); lazy (nothing updated until a refresh at the end).
- Result:
  - Build: 21.6 ms reading 158,915 bytes over 675 fragments, against 24.5 ms reading 214,129 bytes.
  - Stream: eager 7.5 ms reading 5,671 bytes in all, exact after all 22 edits; full 481 ms reading 4,780,872 bytes. The 64-file edit costs 1.55 ms and 117 bytes eager, against about 21 ms and 216 KB full.
  - Lazy staleness: after the 64-file edit, 64 file summaries wrong, top 20 words by count still 20/20, by document frequency 19/20, 3 document-frequency errors, dollar total off by $160,000. After all 22 edits: 105 files wrong, top 20 19/20, 47 errors, dollar total off by $222,240. Refresh: 5.9 ms reading 5,475 bytes, exact.
- Verdict: **measured**. Keeping summaries per fragment is cheaper than keeping them stale: eager upkeep costs about 0.3 ms per edit here and is always exact; a shared-fragment edit costs one fragment read whatever its fan-out. Lazy upkeep looks right on top-word lists and is wrong on totals.

### E183 goal-queries (insight queries that read only the fragments a goal needs)
- Branch / parent: exp/162-fragment-options / E182
- Question: a goal-scoped question ("for each file that mentions W, the dollar amounts and dates in the blocks that mention it") needs only some fragments. With E182's per-fragment word sets, how much less is read than scanning files?
- Change: none in the kernel. In `examples/fragment_insights.rs`: 12 words chosen by a fixed seed among words in 5 to 120 files. Each query answered by reading every file, or by reading only the fragments whose word set holds W; answers compared. A second query answered from summaries alone: files that mention W and cite over $100,000 in all.
- Result: 14 files per query on average. Fragments: 0.33 ms reading 8,406 bytes per query, against 15.0 ms reading 217,693 bytes; answers equal for all 12. Summary-only query: 0.025 ms reading nothing, against 18.3 ms reading 214,129 bytes; 3 matches, equal.
- Verdict: **measured**. A goal-scoped query over fragments reads about 4% of the bytes and is about 45 times faster here; a query whose answer is a function of summaries reads no content at all.

### E184 table-aggregates (aggregates kept fresh from the computed-table IR)
- Branch / parent: exp/162-fragment-options / E183
- Question: E180's IR carries an input edit to every computed cell. Can corpus aggregates over the 24 workbooks (dollar totals per workbook and per column header, percent cells counted) be kept fresh by re-evaluating only the edited workbook's IR?
- Change: none in the kernel. `scripts/table_aggregates.py`: aggregates from each workbook's committed csv, checked equal to the IR's. For each of E180's 22 input edits, the truth is LibreOffice's recalculated csv for that workbook with the other 23 unchanged. Policies: full (parse all 24 csv renders); ir (re-evaluate the edited workbook's IR and apply the difference of its aggregates); lazy (left as they were).
- Result: ir and full both exact 22/22. ir 0.14 ms against 0.32 ms (one workbook evaluated against 24 renders parsed, 4,412 bytes; the corpus is small). Lazy: 18 of 22 edits leave an aggregate wrong; median error in the corpus dollar total $1,950,000 of $93,589,350.
- Verdict: **measured**. The IR keeps table aggregates exact from one workbook's evaluation with no renderer run. At 24 small workbooks the time saved is small; the point is that it scales with the edited workbook, not the corpus.
- Gates (E182 to E184): examples and scripts only; clippy `-D warnings` and fmt clean; the kernel is as at E169's gates.

### E170 hybrid-store (content in packs, refs and index in RocksDB)
- Question: on the Mac (E122's run of Nov 9) packs flushed a commit in 10.8 ms and RocksDB in 1.4 ms, and packs opened 1M objects in 69 ms. Does keeping content in packs but refs, or refs and the object index, in RocksDB get RocksDB's commit with packs' storage?
- Change: `backends::hybrid` (feature `store-rocksdb`, off unless built with it): `RefsHybrid` is a PackStore for objects and a RocksDB for refs; a ref update syncs the pack, then writes the ref in one synced batch. `IndexHybrid` appends objects to its own packs and keeps each object's place (pack, offset, length) in RocksDB; a commit syncs the pack data, then writes the places and the ref in one synced batch, so a swap that adds no object is one flush. Reads check the id and CRC. `examples/stores.rs` gains `hybrid-refs` and `hybrid-index`; `tests/backends.rs` runs the store contract on both.
- Result (Linux, ext4, the harness's db, crash and 1M phases):
  - Single commit mean: packs 5.72 ms, RocksDB 4.90, hybrid-refs 5.30, hybrid-index 5.99. Ref swap p50: 464, 160, 189 and 162 us. A batch of 10k: 4.2, 4.7, 3.8 and 7.5 s.
  - Flushes per commit (strace over 2,010 commits): packs 2 and a rename, RocksDB 1, hybrid-refs 2, hybrid-index 1.5 (swaps skip the pack sync). The hybrids do not cut a commit's flushes: content and ref still live in two files.
  - Kills: 50 of 50 clean on both (7,411 and 5,914 events), nothing acknowledged lost.
  - 1M objects: build 2.3 s packs, 11.7 RocksDB, 2.4 hybrid-refs, 4.2 hybrid-index; on disk 372, 590, 372 and 380 MB; open 205, 966, 252 and 663 ms; cold read p50 38, 50, 37 and 49 us; a missing id 0.2, 45, 0.2 and 5.8 us.
- Verdict: **kept** as options, built only with `store-rocksdb`; not a winner. hybrid-refs costs RocksDB's build and open for faster ref swaps; hybrid-index gets one flush only on swaps and opens three times slower than packs. E172 gets one flush a commit without RocksDB.

### E171 lazy-pack-open (open the active pack's checkpoint in place)
- Question: a PackStore open reads every sealed index's fanout, but loads the open pack's whole checkpoint into a map: at 1M objects 205 ms on Linux and 69 ms on the Mac. Can open use the checkpoint where it lies, as a sealed index is used?
- Change: `PackOptions::lazy_checkpoint` (off by default) with `PackStore::open_with`. The checkpoint is already a sorted index with a fanout; open checks its size, magic, fanout and coverage (not its hash) and searches it in place, scanning only the records after it. A checkpoint is then synced when written, and folded with the newer entries on the next checkpoint or seal. A torn or short checkpoint falls back to a full scan. Nothing stored changes: the files are the ones the default writes. `tests/backends.rs`: the contract with the option on; a lazy view equals a full load after deletes and replaces; a torn checkpoint falls back.
- Result (Linux, 1M objects, three probes each):
  - Open: 186 to 237 ms to 0.9 to 1.2 ms.
  - Cold read p50 36 to 42 us becomes 78 to 80 us (one more read, into the checkpoint, until the page cache holds it); warm read p50 the same (30 to 38 us). A missing id 0.2 to about 2.5 us.
  - Build of 1M: 2.5 to 4.4 s (each checkpoint is synced and rewritten whole).
  - Kills (with E172's ref log, `packs-fast`): 50 of 50 clean.
- Verdict: **kept** as an option, off by default; on its branch until Eric says to merge. Worth turning on for a server that restarts often or opens many stores; a long-running server pays the slower cold reads instead.

### E172 pack-ref-log (refs as records in the open pack: one flush a commit)
- Question: a PackStore commit is two flushes and a rename: the pack's data, then the ref file (written, synced, renamed). On the Mac each flush is an F_FULLFSYNC, which is most of the 10.8 ms. Can the ref ride in the same flush as the objects it names?
- Change: `PackOptions::ref_log` (off by default). A ref update appends a ref record (a marker id, then the name and new id) after the objects, and syncs the pack once; no ref file is written. `refs.snap` keeps all refs at each checkpoint and seal (not synced: losing it only means a longer replay); open loads it and replays ref records after it. A store that kept refs as files carries them over on first open. A record torn ahead of a ref cuts the ref with it, so after a crash a ref only ever falls back to an earlier acknowledged value. Separately, in every mode, sealing a pack now syncs the packs directory after creating the next pack file (it did not before). `tests/backends.rs`: the contract with the option, alone and with E171; refs replay with no checkpoint or snapshot; a flipped byte in an object cuts it and the ref after it; refs carried over from files.
- Result (Linux):
  - Flushes per commit (strace over 2,010 commits): 2 and a rename to 1 and none. Files opened 10,068 to 3,028.
  - Single commit mean 5.26 to 4.74 ms (on this disk a flush is about 40 us, so the time is mostly the harness's writes); ref swap p50 495 to 143 us, p99 1,356 to 310 us. RocksDB's is 160 us.
  - Kills: 50 of 50 clean, alone and with E171 (7,672 and 7,682 events), nothing acknowledged lost.
  - 1M objects: the same bytes on disk (+886 bytes), open as before (190 to 246 ms; 1.5 ms with E171), first ref 0.12 to 0.19 ms.
- Verdict: **kept** as an option, off by default; on its branch until Eric says to merge. A stored-format addition (ref records and `refs.snap`), so turning it on for his stores needs his yes; a store written with it needs a kernel that knows the records. Expect it to halve a commit's flush time on the Mac; that is a Mac measurement.

### E173 scale-10m (10M objects: packs, packs with E171 and E172, RocksDB, both hybrids)
- Question: E122 measured 10M objects before E170 to E172. Do the new options and the hybrids hold up at 10M, and does lazy open still pay there?
- Change: none in the kernel; the harness's build and probe phases at 10M (about 3.7 GB of small objects), each store built, probed three times and deleted before the next. `packs-fast` is PackStore with `lazy_checkpoint` and `ref_log`. The machine has 15 GB of memory and the probes do not drop the page cache, so "cold" reads here are partly cached; the Mac run should drop it.
- Result (Linux, ext4):
  - Build: packs 75 s, packs-fast 93, RocksDB 236, hybrid-refs 77, hybrid-index 157.
  - On disk: packs and packs-fast 3.72 GB, hybrid-refs 3.72, RocksDB 4.32, hybrid-index 4.31.
  - Open: packs 329 to 444 ms; packs-fast 2.0 to 2.5 ms; hybrid-refs 276 to 360 ms; RocksDB 3.6 s on the first open after the build, then 22 to 31 ms; hybrid-index 3.0 s, then 18 ms.
  - Cold read p50: packs 146 to 162 us, packs-fast 191 to 208, RocksDB 122 to 209, hybrid-refs 156 to 165, hybrid-index 107 to 174. Warm read p50: packs 40 to 45 us, packs-fast 42 to 49, RocksDB 83 to 99, hybrid-refs 44 to 49, hybrid-index 80 to 88.
  - A missing id p50: packs 142 to 148 us (a sealed index has no filter), RocksDB 161 to 182, hybrid-index 69 to 101.
- Verdict: **measured**. PackStore with E171 and E172 is the best all-round store at 10M: the smallest, three times faster to build than RocksDB, open in 2 ms, warm reads twice as fast as RocksDB's; it gives up about 40 us on cold reads. RocksDB's open is fast only after it has compacted. A filter on sealed indexes would cut missing-id lookups (a next experiment). Repeat on APFS on the Mac with the page cache dropped.

### Gates for E170 to E173 (Nov 10)
- On exp/173-scale-10m (E172 plus docs): 176 kernel tests pass, 182 with the store features; clippy `-D warnings` clean with and without them (E171's lazy index is boxed so `examples/goal.rs`'s store enum stays small; exp/171 alone predates that fix); the fixed bench matches main (badc682) in every byte count and the north score (0.3035); cost score 0.2430 against 0.2486, from timings only.
- `packages/kernel/scripts/mac-bakeoff.sh` gains two named steps, outside the default run: `fullfsync` builds a second stores binary with RocksDB on F_FULLFSYNC, in its own target folder; `night` measures commit cost (both RocksDB builds), kills and 1M probes for packs, packs-log, packs-fast, RocksDB and the hybrids (`NIGHT_OBJECTS="1000000 10000000"` adds 10M). On Darwin the script now points dyld at the Command Line Tools' libclang, so RocksDB builds through the asdf shim. Dry-run here with small sizes.

### E174 kill-10m (the one-flush commit killed at random over a 10M-object store)
- Question: E172's 100 clean kills were on small stores. Does a commit with refs in the pack stay crash safe on a store that already holds 10M objects, sealed packs and a checkpoint, and how fast does it reopen after a kill?
- Change: none in the kernel. Two harness phases. `crash-big` runs E170's database crash child (through the Repo) over a store of N contents; at 10M a fresh Repo handle spends its whole kill window on its first write (it reads every stored content for the seed index, and `is_needed` reads the whole store before each drop; E129 and E131 have options for both), so it measured nothing. `crash-store` (on exp/176, where it was committed) is the same test on the store alone: a child writes 1 to 8 objects (100 bytes to 200 KB), a commit object naming them and the previous commit, moves `heads/log` and acknowledges; it is killed at a random moment. After each kill: the reopen time, the ref at or past the last acknowledged commit, the last 2,000 commits and every object they name read back and hash to their ids, and 100,000 of the 10M seed objects read back byte for byte.
- Result (Linux, ext4, 10M seed objects, 30 kills each):
  - packs-fast (lazy checkpoint and ref log): 30 of 30 clean, nothing acknowledged lost, 36,609 commits in the kill windows; reopen p50 16 ms, max 83 ms.
  - packs (defaults): 30 of 30 clean, 20,178 commits; reopen p50 13 ms, max 294 ms (a reopen right after a seal scans the new pack).
  - Objects checked after the kills: 264k and 253k, all matching their hashes.
- Verdict: **measured**. The one-flush commit holds up at 10M and makes 1.8 times the commits in the same time. The Repo above the store is the next bottleneck on a big store (E178).

### E175 pack-filters (a Bloom filter beside each pack index)
- Question: at 10M a lookup searches the open pack's index, then each sealed pack's, newest first; a missing id costs a read in every one (142 us at 10M in E173), and a hit in an old pack pays for every newer one first. Does a filter per index close the cold-read gap to RocksDB?
- Change: `PackOptions::filters` (off by default). Sealing a pack writes `NNNNNNNN.flt` beside its `.idx` (10 bits an id, 7 probes, about 1% false positives, a BLAKE3 trailer); a checkpoint with `lazy_checkpoint` writes `.cflt` for the open pack's saved index. A lookup skips an index whose filter says no. A filter that is missing or fails its check is rebuilt from its index at open, so the files are derived and never needed for correctness. `tests/backends.rs` `pack_store_filters`: the contract with the option, alone and with every option; 20,000 objects through a lazy checkpoint; a damaged filter rebuilt; a store written with filters opened without them. In-module: no false negatives.
- Result (Linux, 10M objects, page cache dropped before each probe, three probes each):
  - Cold read p50: packs 155 to 181 us, with filters 77 to 84 us; packs-fast 163 to 215 us, with filters 78 to 115 us.
  - A missing id p50: 121 to 168 us without, 0.5 to 0.9 us with.
  - Open: packs 289 to 396 ms, with filters 287 to 413 (the filters are read whole: 12.5 MB); packs-fast 2.4 to 59 ms, with filters 23 to 32 ms.
  - Disk: 10.6 MB more (0.3%). Build faster with filters (35 s against 76 s, 41 against 97 s for packs-fast), because each write's "is it stored" check no longer reads every sealed index.
- Verdict: **kept** as an option, off by default. It halves cold reads at 10M and makes missing ids almost free; with E171 and E172 it is about as fast cold as RocksDB was in E173 (122 to 209 us). The filter files are a stored-format addition, though derived and rebuilt when missing, so turning it on for his stores needs his yes. A prefetch of the indexes was not tried: at 10M they are 440 MB, more than prefetching should cost at open, and with filters a cold read is already one index probe and one record read.

### E176 pack-compact (a pack cleaner, and its time at 10M)
- Question: packs only grow; a replaced or deleted object stays until something rewrites the pack. What does cleaning cost at 10M?
- Change: `PackStore::compact()` (called explicitly; nothing runs it on its own). It seals the open pack, copies the newest live record of each id out of the old packs into new ones (checking each record's CRC on the way), seals them, writes `refs.snap` when the ref log is on, syncs the directory, and only then deletes the old packs, oldest first, the pack before its index and filter, then syncs again. A kill at any point leaves either the old packs or both, and both read the same. Harness phase `churn`: delete 3 in 10 objects, replace 1 in 10 with new bytes, compact, reopen and check every 97th object. `tests/backends.rs` `pack_store_compact`: three option sets over four rounds of writes and deletes with compactions between, and simulated kills that leave old packs behind.
- Result (Linux, 10M objects):
  - packs-fast with filters: churn 22 s, compact 55 s, 4.33 GB to 2.61 GB, six packs to five; reopen 9 ms; 0 of the sampled objects wrong.
  - packs: churn 29 s, compact 124 s, 4.31 to 2.60 GB; 0 wrong. The filters make the cleaner twice as fast, since it asks for each id where its newest record is.
  - At 200k: 83.5 MB to 52.2 MB in 0.7 s.
- Verdict: **kept** as a method, run by hand; nothing calls it. It needs the single writer the store already requires. A background or incremental cleaner (a pack at a time) is a later step.

### E177 group-commit (many writers sharing one flush)
- Question: a PackStore has one writer (`&mut self`); a server with many clients serializes their commits, each paying a whole flush while the others wait. With refs in the pack (E172), can one flush cover every commit that is waiting?
- Change: `PackStore::stage_ref` (moves a ref and appends its record without a flush; needs the ref log) and `backends::GroupCommit`, a wrapper for one store shared by threads. A commit appends its objects and ref record under the lock; the first waiter that finds no flush running becomes the flusher, flushes outside the lock (a cloned handle to the open pack), and wakes every commit its flush covered. A commit returns only after a flush that began after its records were appended. When a seal or checkpoint is due, the flusher syncs again first, since records may have been appended during its flush and an index or snapshot must name only durable records. Nothing changes for a store not wrapped. Harness phases `writers` (each writer flushing alone under a shared lock, against group) and `crash-group` (writers sharing flushes, killed at random; after each kill every writer's ref is at or past its last acknowledged commit and its last 2,000 commits read back). `tests/backends.rs` `pack_store_group_commit`: eight threads, 300 commits each, a stale expected value refused, every chain whole after a reopen; a store without the ref log refused.
- Result (Linux, ext4, a flush about 0.2 ms; 2,000 commits a writer, each 1 to 4 objects of 100 to 2,000 bytes; three runs):
  - Commits a second, alone against group: 1 writer 3,000 to 4,300 against 3,100 to 4,900; 2 writers 3,200 to 3,900 against 5,100 to 5,500; 4 writers 3,500 to 4,200 against 5,400 to 7,200; 8 writers 3,800 to 4,600 against 8,900 to 11,200; 16 writers 3,800 to 4,300 against 12,500 to 15,500.
  - Commits a flush: 1.0, 1.4, 2.2, 4.1 and 8.0 for 1 to 16 writers.
  - Latency p99: alone grows with the queue (0.6 ms to 73 to 103 ms at 16 writers); group stays 0.5 to 2.7 ms. Group p50 is higher (0.2 to 1.2 ms), since a commit waits for the flush in progress before its own.
  - Kills (8 and 2 writers, 30 each): 60 of 60 clean, nothing acknowledged lost.
- Verdict: **kept** as an option (a wrapper nothing uses yet). Three times the commits at 16 writers here, and the p99 falls from about 80 ms to 2 ms. On the Mac a flush is an F_FULLFSYNC of several milliseconds, so the gain should be larger; the `night2` step measures it. `ikam serve` would need to share one store between requests to use it.

### E178 repo-cost (one database commit through the Repo, against store size)
- Question: E174's Repo-level kill test made almost no commits at 10M. What does a commit cost through the Repo on a fresh handle as the store grows, and do the E129 and E131 options remove it?
- Change: none in the kernel. Harness phase `repo-cost`: a store of N small contents (put through the Repo), then on a fresh handle ten commits of one 50 to 250 KB content, 1 to 40 events and a `heads/db` move: with the defaults, with `needed_index` (E129), and with that and the seed scope set to nothing (E131).
- Result (Linux, packs-fast; first commit, then p50 and max of the next nine):
  - Defaults: 10k 10 ms, 11, 26; 100k 5 ms, 6, 175; 1M 5 ms, 10, 2,245 ms. Most commits are cheap, but one that drops a superseded copy reads the whole store first (`is_needed`), and that read grows with the store: 2.2 s at 1M, so about 22 s at 10M.
  - `needed_index`: the first commit builds the index from one pass over the store (81 ms, 665 ms and 7.7 s), then 13, 14 and 13 ms with no spikes.
  - `needed_index` and an empty seed scope: 6, 9 and 6 ms first, then 7, 10 and 8 ms, max 11 ms at every size.
- Verdict: **measured**. The store is not the bottleneck at scale; the Repo's whole-store passes are. For a database writer (`ikam live`, the db layer), `needed_index` with a seed scope keeps a commit near 10 ms at 1M. The one pass `needed_index` makes on a fresh handle (7.7 s at 1M) is the next thing to remove: saving the needed set with the store, or keeping it current in a projection, would make the first commit as cheap as the rest.

### E179 ckpt-after-crash (a scanned tail is saved by the open that scanned it)
- Question: E177's kill test reopened in 460 ms p50 on a 455 MB open pack, where E174's reopened in 16 ms. Why?
- Finding: an open that had to scan records after the last checkpoint (the process died before its next one) set `ckpt_at` to the end of the scan, as if those records were saved. So neither that handle's drop nor its next sync wrote a checkpoint until 64 MiB more had been appended, and a store killed again within each 64 MiB rescanned an ever longer tail on every open. This is on main too, in every PackStore mode; E174's commits were large enough to pass 64 MiB between kills, which hid it.
- Change (kernel; no format change): `ckpt_at` starts where the saved checkpoint ends; an open that scanned 64 MiB or more syncs the pack and writes a checkpoint before returning. `tests/backends.rs` `pack_store_saves_a_scanned_tail` (fails before the change): 70 MiB written, the checkpoint removed as if the process had died, the next open saves one; with the defaults and with the lazy checkpoint and ref log.
- Result: E177's kill test again (8 writers, 30 kills): reopen p50 460 to 35 ms, max 830 to 66 ms; 30 of 30 clean. crash-store at 100k: 10 of 10 clean.
- Verdict: **kept** as a fix, on its branch until Eric says to merge. It changes no stored format; an open after a crash may now write one checkpoint.

### Gates for E174 to E179 (Nov 10)
- On exp/179-ckpt-after-crash (E174 to E179 stacked on exp/173): 181 kernel tests pass, 187 with the store features; clippy `-D warnings` clean with and without them; the fixed bench matches main (badc682) in every byte count and the north score (0.3035), with only timing fields different. `cargo fmt` was run on the changed files (backends.rs was already off-format on exp/173).
- Branches: exp/174-kill-10m (crash-big phase), exp/175-pack-filters, exp/176-pack-compact (also the crash-store phase), exp/177-group-commit, exp/178-repo-cost (and the Mac `night2` step), exp/179-ckpt-after-crash (the fix, these docs). Each builds on the one before; nothing merged.
- `packages/kernel/scripts/mac-bakeoff.sh` gains `night2`, outside the default run, with `BRANCH=exp/178-repo-cost` or later: store-level kills at 1M, filters and the cleaner at 1M, writers alone and grouped at 1, 4 and 16 threads, group kills, and Repo commit cost at 10k and 100k.

### Merge of E170 to E179 into main (Nov 10)
- On Eric's word ("Merge the appropriate changes"), exp/179-ckpt-after-crash (E170 to E179) merged onto main after the fragment merge (5a535de), with every option off by default: `PackOptions` `lazy_checkpoint`, `ref_log` and `filters` stay false, `compact` and `GroupCommit` run only when called, the hybrids build only with `store-rocksdb`, and FsStore stays the default store. The E179 fix (an open that scanned 64 MiB or more after a crash saves a checkpoint) applies to every PackStore. The log and registry kept both sides; the registry is in experiment order.

### Mac night2 on main (b27e9bf, Nov 10, from the Mac session's report)
- With full fsync on both, packs-fast commits in 4.9 ms against RocksDB's 5.9 ms; packs-fast opens 1M objects in 0.2 ms; 16 writers sharing a flush make 2,550 commits a second against 317 alone, p99 103 to 10 ms; every crash test clean.
- One step skipped: `churn packs-filter` panicked with "churn is for PackStore backends" (E185).

### E185 churn-all-packs (one pack options table for every harness phase)
- Finding: `examples/stores.rs` `churn` kept its own list of pack names (packs, packs-fast, packs-fast-filter) and missed packs-filter, so the Mac night2 churn step for filtered packs panicked.
- Change (harness only; kernel untouched): `pack_options(name)` maps every `packs*` name to its `PackOptions`, and `open` and `churn` both read it. The stores example now runs its own tests (`test = true`): every `packs*` name in `BACKENDS` has options, and build plus churn at 2,000 objects runs on all six. The churn test fails on the old list.
- Gates: 187 kernel tests, 195 with the store features; clippy clean with and without them; fmt clean; the bench is unchanged (the kernel is unchanged).
- Verdict: **kept** as a fix, on exp/185-churn-all-packs until Eric says to merge. The Mac reruns `churn packs-filter` at 1M once Remote Control is back.

### E186 stored-summaries (per-fragment summaries stored as derivations beside the fragments)
- Branch / parent: exp/186-stored-summaries / main (a890aa6, E182 to E184)
- Question: E182 kept summaries in memory, so a restart rebuilds them from every fragment. Stored as derivations beside the fragments, do they survive restarts, gc and crashes exactly, and what do they cost to keep against eager in-memory and lazy summaries?
- Change: none in the kernel. `functions/summary/` is a no_std WASM summary function (8,375 bytes; words of three or more alphanumerics, dollar amounts, dates, headings and list items, ported from E182's `Summary::of`), output as LEB128 varints. It equals the native summary on all 2,008 blocks of the corpus (280 ms for all). `examples/stored_summaries.rs`: each fragment's summary is `repo.apply(summary.wasm, fragment)`, recorded in the ledger; a view graph labels `f/<fragment>` to its summary; each commit's tree is `files` plus `summaries`, so one ref swap moves both and gc keeps live summaries. Summaries are keyed by content id, so they can be missing but never stale. `examples/common/insights.rs` holds the corpus and edit code shared with `fragment_insights.rs` (whose output is unchanged). Modes: memory (E182's cache), stored (WASM derivations), native (the same summary bytes put as plain content, no derivation), and stored with postings (E187). Every write mode turns on `needed_index`, an empty seed scope with `replace_hints`, and `read_memo` after the corpus is stored. The corpus is E182's: 407 files, 214,129 bytes, 675 fragments; 22 edits; FsStore, PackStore and MemStore.
- Result:
  - Exact: after every one of the 22 edits, the view names exactly the head's fragments and every summary equals the native function's bytes, in every mode and store; then gc (2 objects deleted, view intact), fsck and fsck deep clean, and again after a restart.
  - Upkeep over 22 edits, MemStore: memory 1.56 ms; stored 50.6 ms (2.3 ms per edit); native 45.4 ms. PackStore: 1.36, 56 and 44 ms. FsStore: 2, 200 and 216 ms (one fsync per object).
  - Written beyond memory over 22 edits: stored 62,993 bytes, native 61,774. Store at the end: memory 226,727 bytes, stored 447,490, native 406,279 (a summary holds every word, so summaries are about as big as the text).
  - First goal query after a restart (FsStore): memory rebuilds, 25 ms reading 120 KB; stored scans summaries, 21 ms reading 163 KB. Warm queries are the same in every mode (0.35 to 0.9 ms).
  - Before `needed_index`, view upkeep read 4.2 MB over 22 edits: `is_needed` scans the whole store for each replaced chunk. With it, 367 KB.
- Verdict: **measured**. Stored summaries are exact through edits, gc and restart, and checkable (each is a recorded derivation of a pure function). WASM costs about what native does. Without an index they do not make queries faster: a summary scan reads as much as rebuilding.

### E187 summary-postings (a word index stored in the view)
- Branch / parent: exp/186-stored-summaries / E186
- Question: a goal query asks which fragments hold a word. Stored next to the summaries, how much does a word index save on the first query after a restart, and what does keeping it cost per edit?
- Change: none in the kernel. In `examples/stored_summaries.rs`, two layouts in the same view graph: lists (`w/<word>` to content holding the sorted fragment ids) and node postings (`w/<word>/<fragment>`, one node per pair, no content). Checked equal to the summaries' words after every edit.
- Result (22 edits): exact in every store. First goal query after a restart: lists 0.73 ms reading 21 KB, node postings 1.24 ms reading 36 KB, against 21 to 25 ms and 120 to 163 KB. Upkeep: lists 319 ms on MemStore, 494 on PackStore, 1,676 on FsStore; node postings 490, 599 and 1,842. Written beyond memory: lists 514 to 518 KB, node postings 558,818 bytes, against 63 KB without postings. Store: about 1.26 and 1.23 MB.
- Verdict: **measured**. Postings make a cold query about 30 times cheaper and an edit about 8 times dearer in bytes written. Lists beat node postings on both counts.

### E188 summaries-kill (kills during view upkeep)
- Branch / parent: exp/186-stored-summaries / E187
- Question: the files and the view move in one ref swap. Does a kill at any point leave a head whose view matches its files?
- Change: none in the kernel. `stored_summaries crash`: a child builds the corpus and loops edits with list postings, committing after each; the parent kills it after 150 to 1,050 ms (fixed seed), reopens, and checks fsck and the full view check on the head.
- Result: FsStore 30 kills, 22 heads checked (8 kills came before the first commit), 57 edits committed; PackStore 30 kills, 30 heads, 51 edits. Every head exact, fsck clean.
- Verdict: **measured**. One commit carries files and summaries, so there is no window where they disagree.

### E189 summaries-scale (stored summaries at 4,500 and 18,000 fragments)
- Branch / parent: exp/186-stored-summaries / E188
- Question: at E186's size every mode is fast. How do upkeep, opening, the first query and storage grow with the corpus?
- Change: none in the kernel. `stored_summaries scale N`: a synthetic corpus (N files of 8 paragraphs from a 20,000-word skewed vocabulary, dollar amounts in a third of paragraphs, a date in a fifth) on PackStore; build, 20 single-file edits (p50 and max), restart (open, ledger load timed apart), first goal query, store bytes.
- Result:
  - 500 files (1.2 MB, 4,500 fragments), upkeep p50: memory 0.97 ms, native 2.96 ms, stored 3.86 ms (24 KB read, 2.1 KB written), list postings 38 ms (452 KB read, 71 KB written).
  - 2,000 files (4.8 MB, 18,000 fragments): memory 1.0 ms, native 7.7 ms, stored 7.1 ms (30 KB read, 2.2 KB written), postings 50 to 68 ms (712 KB read, 122 to 135 KB written).
  - First query after a restart at 2,000 files: memory 472 ms reading 3.2 MB; stored 548 ms reading 5.2 MB; postings 29 to 39 ms reading 30 KB.
  - Store at 2,000 files: memory 3.6 MB, native 8.7 MB, stored 9.7 MB, postings 34.1 MB.
  - The first run found PackStore opens slow with many short derivation records: the ledger load took 142, 333, 1,069 and 4,251 ms at 500 files (memory, native, stored, postings). That is E190; with it, 11, 19, 36 and 93 ms.
  - The first edit that drops a fragment builds `needed_index` (one pass over the store, up to 9.6 MB read); the p50 leaves it out.
- Verdict: **measured**. Stored summaries cost about 7 ms and 2 KB per edit at 18,000 fragments, but save nothing on queries without an index. Postings make the first query 12 to 16 times faster and cost 35 to 60 times the bytes per edit and 3.5 times the storage.

### E190 pack-prefix (prefix lookups in the open pack without a scan)
- Branch / parent: exp/190-pack-prefix / main (a890aa6)
- Finding: E189 (exp/186-stored-summaries) opened a PackStore of 38,862 objects, most of them short derivation records, in 4.25 s of ledger load. The ledger resolves each short record's abbreviated ids through `ids_with_prefix`, and PackStore answered that for the open pack by scanning its whole index, so the load was quadratic in the open pack. Writing a short record runs the same lookup.
- Change (kernel; no format change): the open pack's index stays a HashMap, so open and point lookups are untouched. A sorted list of its ids is built on the first prefix lookup and answered by binary search, with ids written since the sort kept in a short unsorted tail that is merged every 4,096 writes; both are dropped when the open pack is checkpointed or sealed. `tests/backends.rs` `pack_store_prefix_lookups_in_the_open_pack_match_a_mem_store`: 2,000 ids with every seventh deleted, prefixes of 0, 1, 2, 5 and 32 bytes, equal to a MemStore, again after reopen.
- Result:
  - E189's ledger load at 500 synthetic files: 142, 333, 1,069 and 4,251 ms (memory, native, stored, postings modes) to 11, 19, 36 and 93 ms. At 2,000 files: 39 to 257 ms.
  - Store level at 200,000 objects (`stores build` and `probe`, packs, two runs each): build 0.3 s, open 30 to 32 ms, point read p50 1.1 to 1.3 us, missing p50 0.1 us; main 0.3 s, 33 to 42 ms, 1.1 to 1.2 us and 0.1 us. A first version that made the open pack's index a BTreeMap was as fast on prefixes but opened in 58 to 67 ms with reads at 1.4 us, so it was replaced.
- Verdict: **kept** as a fix, on its branch until Eric says to merge. No trade-off measured, so no option.
- Gates: 188 kernel tests, 196 with the store features; clippy `-D warnings` clean with and without them; fmt clean; the fixed bench matches main (a890aa6) in every byte count and the north score (0.3035).

### E191 bucketed-postings (a word's list split by fragment id)
- Branch / parent: exp/186-stored-summaries / E189
- Question: postings rewrite a word's whole list when one fragment changes. Split by the first 1 or 2 hex digits of the fragment id (16 or 256 buckets), does an edit write less?
- Change: none in the kernel. Modes `stored+postings/16` and `/256` in `examples/stored_summaries.rs` (`w/<word>/<prefix>` to content); exact checks as E187.
- Result: exact in every store, through gc and restart. Written per edit (p50) at 500 files: 70 KB lists, 76 KB with 16 buckets, 78 KB with 256. At 2,000 files: 135, 66 and 101 KB, but the first query slows from 39 to 105 and 78 ms and the store holds 2 to 3 times the objects. On E186's corpus: 517, 593 and 618 KB over 22 edits.
- Verdict: **discarded**. The cost of postings is the number of words an edit touches (each a node rewritten in the view's tree), not the length of their lists. Fewer writes need fewer words per fragment or a delta log merged in the background (database mode), not smaller lists.

### Gates for E186 to E191 (Nov 11)
- On exp/186-stored-summaries: 187 kernel tests pass; clippy `-D warnings` and fmt clean; the fixed bench matches main (a890aa6) in every byte count and the north score (0.3035): the kernel is unchanged. E189 and E191 numbers were measured with E190 merged in locally (`trial` branch, not pushed); without it, PackStore opens of the postings modes take seconds.
- Run: `cargo run --release --example stored_summaries` (E186, E187, E191; about 80 s), `-- crash` (E188), `-- scale 500` and `-- scale 2000` (E189, E191).

### Merge of E186 to E191 into main (Nov 11)
- On Eric's word ("Merge both"), exp/190-pack-prefix (E190) and then exp/186-stored-summaries (E186 to E191) merged onto main a890aa6. The E190 fix applies to every PackStore and changes no stored format. Stored summaries are an example (`examples/stored_summaries.rs`, `functions/summary.wasm`) and are off unless an application commits the view. The log and registry kept both sides, in experiment order.

### E192 ir-templates (cells and line templates: an opt-in IR codec)
- Branch / parent: exp/192-ir-compression / E191 (main 17589ee, with E186 to E191 merged)
- Question: Eric asked (Nov 11) how intermediate representations can find connections that aid dedup and compression. The kernel already templates bytes at repack (`template.rs`, E143, E144), and xz finds repeated text by itself. Does a format-aware IR (cells, and the skeleton around them, with each distinct line skeleton stored once) beat xz, gzip, zstd and the kernel on report-like files, and what else does it buy?
- Grounding: E161 and E165 (a typed TableIR stores the three text formats in about half the bytes); LogShrink (`docs/research/kernel-direction.md` #15: a log line is a template plus variables, the variables stored by column); the Mercante report-copy demo (30 synthetic reports: 0.52 of raw in the live store before repack, 0.16 after repack, gzip -9 0.14, xz -9 0.11). The demo's 52% was the unrepacked store, so the gap to xz is 1.45x, not 4x.
- Change:
  - `src/ir.rs` (new, `pub mod ir`; nothing in put, repack or the bench calls it). Lexers: csv fields; json values (keys stay skeleton); md pipe-table cells; numbers in any other text. The skeleton is cut into lines at its newlines, and each distinct line skeleton is stored once. Cells go to token streams: the previous value again, a number as a delta, a string seen before by index, or a new string. Options (all off means templates with cells in file order): `columns`, `numbers`, `rows`, `siblings` (E193, E194). `decode` gives back every file byte-exact.
  - `tests/ir.rs`: all 16 option combinations round-trip a hand-made set (quoted csv, escaped json, md tables, a NUL, files without a final newline) and arbitrary bytes by property (3,000 cases); a unit test pins the number forms (`-$1,650,000.50`, `12.5%`; `007`, `-0`, `1,23` stay strings).
  - `examples/ir_compress.rs`: the corpora, baselines and measurements for E192 to E196. Every method is checked byte-exact; the kernel rows also pass fsck.
- Corpora: `reports-drift`, 300 synthetic lead-lag reports modelled on mercante-personal's `leadlag/cli.py render()` (header, settings prose, an 11-column link table, paths, links), whose numbers move only when a month of data arrives (about every 20 runs), 1,302,537 B; `reports-random`, the same with every number redrawn each run (worst case), 1,270,467 B; `reports-30`, 30 random runs (the demo's shape), 79,535 B; `exports`, 100 drift runs with each table also as csv and json, 558,615 B; `tables`, `fixtures/tables` csv, json and md (24 real workbooks, 72 files), 19,624 B; `texts`, `fixtures/multiformat` md (24 prose texts, a control), 99,212 B.
- Result, bytes (against xz -9 over all files as one stream):

| method | reports-drift | reports-random | reports-30 | exports | tables | texts |
|---|---|---|---|---|---|---|
| gzip -9, one stream | 31,882 (1.891) | 92,950 (1.368) | 6,708 (1.221) | 17,905 (1.630) | 5,582 (1.115) | 25,856 (1.128) |
| xz -9, one stream | 16,856 | 67,936 | 5,496 | 10,984 | 5,008 | 22,932 |
| zstd -19, one stream | 16,984 (1.008) | 69,581 (1.024) | 5,620 (1.023) | 11,027 (1.004) | 5,185 (1.035) | 23,559 (1.027) |
| zstd 3 + dictionary per file (bench baseline) | 201,343 (11.95) | 244,518 (3.599) | 14,181 (2.580) | 109,385 (9.959) | 10,097 (2.016) | 36,823 (1.606) |
| kernel, one graph, archive weight, repacked | 27,657 (1.641) | 89,330 (1.315) | 6,551 (1.192) | 22,036 (2.006) | 6,508 (1.300) | 25,308 (1.104) |
| IR templates, xz | 17,404 (1.033) | 59,552 (0.877) | 5,584 (1.016) | 7,612 (0.693) | 4,684 (0.935) | 28,652 (1.249) |

  - Times (fastest of 3, ms, encode / decode): xz -9 on reports-drift 248 / 3.6; IR templates + xz 104 / 14; kernel ingest and repack 3,400, all reads 9.6.
  - The IR over the 300 drift reports is 65 line skeletons; their layout is 4,404 B under xz.
- Verdict: **measured**. Templates alone roughly tie xz on reports (xz already finds repeated layout in one stream) and win where the same layout recurs across formats (exports 0.693). On prose they lose 25% (texts), so the IR must be chosen per file type, never applied to everything. The gains come from E193 and E194.

### E193 ir-columns (cells by column, numbers as deltas, rows keyed)
- Branch / parent: exp/192-ir-compression / E192
- Hypothesis: cells of one slot look alike, so streams per (line skeleton, slot) compress better than cells in file order; numbers stored as deltas against the previous number in their stream cost less than their text; and keying a table row's streams by its first cell (the link id) puts the same row of successive runs side by side, so an unchanged value is one token (source: LogShrink observation 2, column layout 36 to 103% smaller than row layout; E161's cell types).
- Change: `ir::Options` `columns`, `numbers`, `rows` (each off by default).
- Result (xz -9 over the pack, against xz -9 of the files; options added in order):

| options | reports-drift | reports-random | reports-30 | exports | tables | texts |
|---|---|---|---|---|---|---|
| templates | 1.033 | 0.877 | 1.016 | 0.693 | 0.935 | 1.249 |
| + columns | 0.926 | 0.717 | 0.917 | 0.662 | 1.082 | 1.291 |
| + numbers as deltas | 0.767 | **0.669** | **0.840** | 0.645 | 1.105 | 1.291 |
| + row keys | **0.591** | 0.677 | 0.905 | 0.661 | 1.312 | 1.327 |

  - reports-drift: 9,968 B against xz's 16,856 (0.591), gzip's 31,882 and the kernel's 27,657. Encode 57 ms (xz alone 248), decode 12 ms (xz 3.6).
  - With row keys the drift reports become 1,853 streams; an unchanged value costs one token, so the gain is largest where runs repeat values, as reruns on the same data do. With every number random (reports-random), row keys add stream overhead and numbers-as-deltas without them is best (0.669).
  - On the real tables (24 small workbooks, 3 rows on average) columns cost more than they save (1.082): too few cells per column.
- Verdict: **measured**. Columns plus numeric deltas are a 16 to 33% gain over xz on reports; row keys add a further 23% when values repeat across runs. Each is an option because the best combination depends on the data: row keys for reruns, no columns for small tables, nothing for prose.

### E194 ir-siblings (one table as csv, json and md, stored once)
- Branch / parent: exp/192-ir-compression / E193
- Hypothesis: a file whose name differs from an earlier one's only by extension holds the same facts; storing its cells as a diff against that file's cells (copy runs plus inserted cells) stores the facts once across formats, a canonical form without a format-specific parser (source: E112's cross-type sharing, E161, E165).
- Change: `ir::Options::siblings` (off by default). The diff is Myers over the two cell lists; inserted cells go to the streams as usual. Lexing makes the cell lists agree without per-format renderers: csv quotes, json keys and md pipes are skeleton.
- Result (xz -9, against xz -9 of the files):
  - exports (each report as md, csv and json): 5,300 B (0.483), from 7,256 without siblings; 0.507 without row keys. gzip 1.630, kernel 2.006.
  - tables (real workbooks): 4,640 B (0.927) with numbers and siblings but no columns, the best of every combination; with columns and row keys 1.059.
  - Corpora without siblings are unchanged.
- Verdict: **measured**. Cross-format canonical storage halves exports against xz. It needs no renderer: the cell lists of csv, json and md agree once the lexers keep quoting, keys and pipes in the skeleton (json leaves out the header, which the diff handles as a skipped run).

### E195 ir-in-kernel (the IR's parts stored as kernel contents)
- Branch / parent: exp/192-ir-compression / E194
- Question: the IR packs above are compressed by xz outside the kernel. Stored in the kernel as four contents (skeletons, files, stream directory, all streams), at the archive read weight and repacked, how do they compare with the kernel storing the files? And with each stream compressed alone by zstd -19 (each column readable on its own)?
- Change: none in the kernel. `examples/ir_compress.rs` stores the best IR per corpus (chosen by the xz size) in a fresh repository: put, commit, repack, read back, fsck.
- Result (bytes, against xz -9; kernel storing the files in brackets):
  - reports-drift: 10,362 (0.615) [27,657, 1.641]: 63% smaller. Ingest and repack 218 ms against 3,400 ms.
  - reports-random: 47,444 (0.698) [89,330]; reports-30: 4,703 (0.856) [6,551]; exports: 5,574 (0.507) [22,036]: 75% smaller; tables: 4,735 (0.945) [6,508]; texts: 27,127 (1.183) [25,308]: 7% larger, the control.
  - zstd -19 per stream (each column alone): reports-random 49,013 (0.721), but reports-drift 37,832 (2.244) and exports 15,560 (1.417), as 674 to 1,853 small frames each pay their own header.
  - Reading one report: the kernel 0.03 ms for the stored file; the IR decodes every file, 12 ms for the 300 drift reports.
- Verdict: **measured**. Stored in the kernel, the IR beats xz on every report and table corpus and stores reports in a third to a quarter of what the kernel stores for the files, with ingest 15 times faster. The price is reads: one report costs a full decode, so this is an archive option (cold report history), or wants streams cut into blocks of runs before it serves reads.

### E196 ir-benefits (what the IR buys beyond bytes)
- Branch / parent: exp/192-ir-compression / E195
- Question (Eric: "be creative when thinking about benefits as well"): what do the connections the IR exposes give besides size? Measured on reports-drift with columns, numbers and row keys.
- Result:
  - **Column query**: one link's dev rho over its 234 runs, from that link's stream alone (zstd -19 per stream): 0.08 ms reading 2,064 B (the stream and the compressed directory), against 0.46 ms scanning the 1.3 MB of text in memory and 3.8 ms from the xz archive (16,856 B, all of it decompressed). Answers equal.
  - **Aggregates**: every link's mean dev rho (24 links): 0.92 ms reading 3,288 B against 1.6 ms scanning the text; equal.
  - **Edit propagation**: one link's mechanism sentence revised in all 218 reports that list it. In the IR one line skeleton changes and every other part is byte-identical. The kernel adds 49,608 B for the 218 edited reports (482 ms; +31,999 B after repack) and 624 B for the edited IR (1.6 ms; +498 B after repack): 80 times fewer bytes.
  - **Drift checks at ingest** (16 ms for 302 reports): a normal report puts at most 2 lines on a skeleton never seen before (a new link), a report whose table gains a column 26; a dev rho typed as 7.312 jumps 394 times its column's largest earlier move, against at most 4 times for every normal report.
  - **Export consistency**: of 100 reports with csv exports, 5 csv files left one cell stale; the md-against-csv cell diff flags exactly those 5 (no false alarms) in 4 ms.
  - **Layout apart from values**: the drift reports' layout is 4,404 B and their values 5,820 B under xz. The layout can be shared, versioned or reviewed without any number, and the values kept private (Mercante's licensed prices are never shared).
- Verdict: **measured**. The IR turns text into addressable facts: column reads, aggregates and consistency checks without parsing reports, a layout edit as one object, and ingest-time checks for layout drift and impossible values.
- Gates (E192 to E196): 191 kernel tests pass (2 new in `tests/ir.rs`, 1 unit test); clippy `-D warnings` and `cargo fmt --check` clean on all targets; the fixed bench matches main (17589ee) in every byte count, read cost, proxy score and the north score (0.3035); cost score 0.2404 against 0.2398, from timings only.
- Real reports: `cargo run --release --example ir_compress dir:<folder>` runs the size table over any folder and prints only sizes and times.

### Merge of E192 to E196 into main (Nov 11)
- On Eric's word ("Merge it"), exp/192-ir-compression merged onto main 17589ee as f46d252 (the merge tree is the branch tree). Every IR option is off: nothing in put, repack or the bench calls `ir`; it is a module an application or an experiment chooses.
- Gates on the merge: 191 kernel tests pass; clippy `-D warnings` and fmt clean; the fixed bench gives the same 147 byte and count fields as main's last two runs (E186, E190) and north 0.3035.

### E197 ir-blocks (read one report without decoding every report)
- Branch / parent: exp/197-ir-blocks / main a831e15 (E192 to E196 merged)
- Question: E195 left one catch: the IR is one solid pack, so reading one of 300 reports decodes all 300 (12 ms, against 0.03 ms for a file stored in the kernel). Can the IR be cut so a read decodes a small part, and what does that cost in bytes?
- Change: `ir::encode_blocks(files, options, per, keyframe)` cuts the files, in order, into blocks of `per`. One skeleton table is shared by every block; each block's streams, dictionaries and sibling diffs are its own, so `ir::reader(&blocks).block(i)` decodes one block. With `keyframe`, each block after the first goes on from the streams as the first block left them (its strings and last numbers), like video frames coded against a keyframe: a cold read decodes the first block and its own, and a reader keeps the first. Each block is stored three ways: xz -9 per block; zstd -19 per block with one trained dictionary (counted); and the kernel, one content per block, repacked at the archive weight. The examples' corpora and compressors moved to `examples/ir_common/` unchanged (`ir_compress` prints the same table).
- Result (stored bytes against xz -9 over all 300 files, and the time to read the middle file, cold):

  | corpus | per block | independent, kernel | keyframe, kernel |
  | --- | ---: | --- | --- |
  | reports-drift | 1 | 1.967, 0.15 ms | 1.943, 0.24 ms |
  | reports-drift | 20 | 1.227, 0.84 ms | 1.129, 1.35 ms |
  | reports-drift | 50 | 0.904, 1.9 ms | 0.858, 3.3 ms |
  | reports-drift | 100 | 0.756, 3.3 ms | 0.718, 6.6 ms |
  | reports-drift | 300 (one block) | 0.636, 9.7 ms | same |
  | reports-random | 20 | 0.938, 1.0 ms | 0.863, 2.3 ms |
  | reports-random | 50 | 0.816, 2.9 ms | 0.792, 4.1 ms |
  | exports (md, csv, json) | 30 | 0.891, 0.58 ms | 0.769, 1.0 ms |
  | exports | 60 | 0.729, 0.81 ms | 0.630, 1.8 ms |

  The kernel is the best home for blocks at every size: its repack deltas reach across blocks where xz and zstd per block cannot (drift, 20 per block: kernel 1.227, xz per block 1.798, zstd with a dictionary 2.150). The files themselves in the kernel are 1.641 with 0.03 ms reads. A read costs about 0.03 ms per report in the block, so the block size is a dial between bytes and read time; the keyframe saves 4 to 14% at the cost of a second block decode on a cold read.
- Verdict: **measured**. Blocks of 20 to 100 reports in the kernel keep most of the IR's gain (0.72 to 1.13 of xz against the files' 1.641) with 1 to 7 ms reads; small blocks (under 10) lose to storing the files. Off unless an application calls `encode_blocks`.
- Run: `cargo run --release --example ir_blocks [reports-drift|reports-random|exports]` (about 6 minutes).

### E198 ir-prose (prose stored as it is)
- Branch / parent: exp/198-ir-prose / E197 (stacked: needs exp/197-ir-blocks)
- Question: the IR lost on prose (texts 1.249 of xz, E192) and a real folder mixes reports with prose notes. Can prose be routed around the IR without losing the reports' gain?
- Change: `Options::prose`: a file under a tenth of whose bytes are cells is stored as it is, in a new raw part (`Parts::raw`, packed only when non-empty, so earlier packs read as before) with the other such files; the rest go through the IR as before. The share separates cleanly on every corpus: prose texts have 2 to 5% of their bytes in cells, the reports 18 to 21%, real tables 14 to 81%.
- Result (xz -9 over the pack, against xz -9 over the files; best IR variant without and with `prose`): reports-drift 0.591 / 0.592; reports-random 0.669 / 0.670; exports 0.483 / 0.482; tables 0.927 / 0.929; texts 1.249 / **1.000**; mixed (the drift reports and the texts in one folder) 1.013 / **0.826**.
- Tried and dropped: (1) keeping a line whole when no other line shares its skeleton: texts 1.225, mixed 0.967, tables 0.964 (worse); with file routing on top it added nothing. (2) Routing a file when most of its lines are such lines: blank lines are shared, so prose files stayed in the IR (texts 1.218). (3) Most of its bytes on lines no other file uses: texts 1.072, but the real tables went raw and lost their sibling gain (1.026). The spec files in the texts share a template of headings and labels, and the IR still loses on them (4,844 against xz's 3,772): xz's long matches already capture a shared template in running text; the IR wins on cells.
- Verdict: **measured**. With `prose`, the IR is never worse than xz on these corpora, and a mixed folder drops from 1.013 to 0.826. Off by default.
- Run: `cargo run --release --example ir_prose` (about 2 minutes).

### E199 ir-nearest (cells diffed against the most similar earlier file)
- Branch / parent: exp/199-ir-nearest / E198 (stacked: needs exp/197 and exp/198)
- Question: `siblings` (E194) diffs a file only against the file with its stem. A rerun of a report shares most of its cells with some earlier run of another name. Does diffing against the nearest earlier file help, and where?
- Change: `Options::nearest` (a window; 0 is off): among the last `nearest` files with the same extension (and the sibling), the one holding the most of this file's cells is chosen as the base, by a cheap count over a cell set, and the cell diff runs on the best two and the sibling. The stored format is E194's (a base and copy and insert runs), so decoding is unchanged. The cheap count keeps the full search's sizes (drift 0.576 both) at 118 ms against 724 ms for diffing all 16; counting cells equal at the same place instead picked worse bases (0.609).
- Result, whole corpus (best IR variant, xz -9 over the pack, against xz -9 over the files; off / window 1 / 4 / 16): reports-drift 0.591 / 0.670 / 0.584 / 0.576; drift in a shuffled order 0.906 / 1.050 / 0.939 / **0.770**; reports-random 0.669 / 0.688 / 0.685 / 0.691; exports 0.483 / 0.561 / 0.465 / 0.466; tables 0.927 / 0.937. Encode 2 to 4 times slower at 16. A window of 1 is worse everywhere: with row keys a value repeated from the previous run already costs almost nothing, and a diff per file costs more.
- Result, blocks in the kernel (E197), best variant (off / nearest 16):

  | corpus | per block | independent | keyframe |
  | --- | ---: | --- | --- |
  | reports-drift | 20 | 0.984 / **0.759** | 0.958 / 0.743 |
  | reports-drift | 50 | 0.885 / 0.700 | 0.839 / 0.690 |
  | reports-drift | 100 | 0.736 / **0.624** | 0.699 / 0.617 |
  | exports | 30 | 0.668 / 0.663 | 0.649 / 0.611 |
  | exports | 60 | 0.564 / 0.526 | 0.555 / 0.523 |

  In small blocks the winner drops row keys (columns and numbers only, E193's TCN): row keys make 1,853 streams that each restart in every block. With nearest, blocks of 20 reports read one report in 0.40 ms (the block decoded in memory) at 0.759 of xz, against the files in the kernel at 1.641 and 0.03 ms. E197's table used row keys for every block size, so it understates small blocks: its 1.227 at 20 per block is 0.984 with columns and numbers alone.
- Verdict: **measured**. Use `nearest: 16` with blocks; whole-corpus packs gain little unless files arrive out of order. Off by default.
- Run: `cargo run --release --example ir_nearest` (about 8 minutes).

### E200 ir-predict (columns that predict each other: generator plus residual)
- Branch / parent: exp/200-ir-predict / E199 (stacked)
- Question: a table often has columns that follow from another column of the same row (a link's sensor and target from its id, a status from its label). Row keys (E193) catch only "follows from the first cell, and repeats from the last run". Can each column learn its own generator and store only where it is wrong?
- Change: `Options::predict` (with `columns`): each column keeps, for each of its line's first 4 cells, a map from that cell's value to the value the column last had beside it, and how often the map held. A cell the best-scoring map gets right is token 5 (one byte); any other cell is stored as before (the residual), and every map learns it. The decoder learns the same maps from the cells it has already decoded, so nothing about the maps is stored. A flag bit tells the decoder to learn.
- Result (xz -9 over the pack, against xz -9 over the files; best variant without / with): reports-drift 0.591 / **0.543** (columns and numbers with predict, no row keys: prediction does row keys' job and more); reports-random 0.669 / 0.654; exports 0.483 / **0.448**; tables 0.927 / 0.926. Decode of 300 drift reports 11 ms to 26 ms (31 ms with every earlier cell as a predictor; 4 kept the sizes). In blocks of 20 in the kernel with nearest 16 (E199) the gain goes: independent 0.759 / 0.760, keyframe 0.743 / 0.734. A block's maps start empty, and 20 reports are too few to learn from; a keyframe's maps carry over and help a little.
- Verdict: **measured**. For whole-corpus packs, `predict` replaces row keys with a better generator (8% smaller on reports, 7% on exports). Not worth it in small blocks. Off by default. The maps are also a learned schema: which column determines which. Reading them out is a next step, not done here.
- Run: `cargo run --release --example ir_predict` (about 6 minutes).

### E201 ir-order (streams laid out by key)
- Branch / parent: exp/201-ir-order / E200 (stacked)
- Question: the IR's streams sit in the pack in the order they first appear, so with row keys (E193) one row's columns sit together and the same column of different rows lies far apart. Does sorting the streams by key (skeleton, slot, row key) give xz closer matches?
- Change: `Options::group`: at the end of an encode, the streams it added are sorted by key and the directory is rebuilt in that order. Decoding is unchanged (the directory names each stream).
- Result (xz -9 over the pack, against xz -9 over the files; without / with): with row keys, drift 0.591 / 0.545, reports-random 0.677 / 0.638, exports 0.661 / 0.632 (with siblings 0.483 / 0.452). Without row keys there are few streams and nothing moves. The best of all variants changes only on reports-random, 0.654 / **0.638**; drift (0.543), exports (0.448) and tables (0.926) stay with predict, which needs no row keys. Blocks of 20 in the kernel: no change (0.759, 0.786, 0.719 / 0.717, 0.998 / 1.000). Predict on top of row keys adds nothing: the row-key streams are already one row's history each.
- Verdict: **measured, small**. Worth it only with row keys on data whose rows repeat in no order. Off by default.
- Run: `cargo run --release --example ir_order` (under a minute).

### E202 ir-schema (the columns read out as a schema, and the cells that break it)
- Branch / parent: exp/202-ir-schema / E201 (stacked; needs only `ir::lex_lines` and `ir::segments`, so it could sit on E192 alone)
- Question: `predict` (E200) learns which column of a line another follows from, only to save bytes. Read out over a folder, is that a usable schema, and does it find errors in the data?
- Change: `ir::schema` (new module; nothing in the codec or the stored format changes). `dependencies(files, min_rows, min_strength)`: for each line template and slot, the slot of the same line it follows from best, counted in file order as the share of lines that repeat the value their key had last time (a key's first line counts as a miss, so a near-unique column is no key), kept when it holds on `min_strength` of them and explains at least half of the slot's changes from line to line. `events(files, deps)`: per key along each dependency, a value that differs from the one before and is back on the next line (a **blip**), one that differs and holds (a **change**), and a key with two values in one file (a **conflict**: two rows share what should be an id; each row is then followed by place).
- Result, schema (min 20 lines, 90%): reports-drift 11 dependencies in 60 ms, and they read as the table's meaning: link determines lag, lead, status and (between monthly reruns) every number; reports-random 2 (lag and the header); exports 21 (the same in md, csv and json); tables 0 (too few rows). The history comes out with it: 914 changes in the drift reports (the monthly reruns' numbers, status changes). It found a real fault in the generated data: two links share the id `fares-to-occupancy-us` (12 conflicts in the exports, 4 in the drift reports).
- Result, errors put in (60 random cells of lines with 2+ cells, each set to another value its column has elsewhere, 3 seeds; the schema read from the corrupted files):

  | corpus | blips on | errors in such a column | found of those | blips flagged that were errors | false blips on the clean corpus |
  | --- | --- | ---: | ---: | ---: | ---: |
  | reports-drift | every dependency | 154 of 180 | 91% | 93% | 5 |
  | reports-drift | dependencies holding 97%+ | 42 of 180 | 98% | 94% | 1 |
  | exports | every dependency | 146 of 180 | 87% | 98% | 0 |
  | exports | dependencies holding 97%+ | 14 of 180 | 100% | 100% | 0 |

  The errors no dependency covers are in columns that change on every run (a run counter, the trial counts in the prose line). The 5 false blips on the clean drift reports are real: numbers that moved in one monthly rerun and came back to the same 3 decimals in the next (checked by hand), and one status that changed and changed back.
- Verdict: **measured**. The IR's templates give a schema and a change history for free, and a blip finder that catches about 9 in 10 errors in the columns it covers with few false alarms. A read-side tool, nothing to switch on.
- Run: `cargo run --release --example ir_schema` (a few seconds).

### E203 ir-query (one column of every report without rebuilding the reports)
- Branch / parent: exp/203-ir-query / E202 (stacked; uses only E192's format, so it could sit on main alone)
- Question: the IR already keeps each column in a stream of its own. Can a question about one column ("every row's status", "one link's status in every report", "rows with dev q under 0.05") be answered from that stream and the files part alone, and how fast against reading and splitting the files?
- Change: `ir::query` (new module; nothing in the codec or stored format changes). `index(parts, skeletons)` reads the files part and the directory only: which skeleton each line of each file has. `column(ix, skeleton, slot, fetch)` decodes that column's stream and pairs each value with its file and line; `row(ix, skeleton, slot, key, fetch)` gives one row's values across files (with row keys, the key column's stream and that row's stream). `fetch` gives a stream's bytes by index, so a store can hand over only the streams asked for. An encode with `siblings`, `nearest` or `predict` is refused: their cells are not all in their own column's stream.
- Result (300 reports; best of 7; each answer checked equal to the one from the files; stored size against xz -9 over the files):

  | corpus, layout | stored | every status | one link's status | dev q under 0.05 | streams read |
  | --- | ---: | ---: | ---: | ---: | ---: |
  | drift, files in the kernel | 1.641 | 17.9 ms | 17.9 ms | 17.9 ms | all files |
  | drift, solid IR (columns, numbers), decoded | 0.767 | 24.8 ms | 24.8 ms | 24.8 ms | all |
  | drift, same bytes, `ir::query` | 0.767 | 1.5 ms | 1.9 ms | 1.8 ms | all decompressed |
  | drift, IR parts in the kernel (columns, numbers) | 0.819 | **0.6 ms** | **1.1 ms** | **1.0 ms** | 1 / 2 / 1 of 23 |
  | drift, IR parts in the kernel, row keys | 1.293 | 1.8 ms | 1.1 ms | 2.2 ms | 26 / 2 / 26 of 1,853 |
  | random, files in the kernel | 1.315 | 25.0 ms | 25.0 ms | 25.0 ms | all files |
  | random, solid IR (columns, numbers), decoded | 0.669 | 25.2 ms | 25.2 ms | 25.2 ms | all |
  | random, IR parts in the kernel (columns, numbers) | **0.643** | **0.6 ms** | **1.1 ms** | **1.0 ms** | 1 / 2 / 1 of 23 |

  The smallest IR without the query's limits (predict, nearest, siblings) is 0.543 of xz on drift: the query costs 1.4 times that there, and nothing on random reports, where the parts in the kernel are smaller than the solid pack (the kernel's own deltas and dictionaries across 23 streams beat one xz window). Row keys make 1,853 small streams, which cost size in the kernel and buy nothing here.
- Verdict: **measured**. Columns and numbers, stored as one kernel content per part, answer a column question in about 1 ms, 18 to 25 times faster than reading the files, at 0.64 to 0.82 of xz. A read-side module; which layout to store is a choice of options already there. Not wired into the store or `serve` yet.
- Run: `cargo run --release --example ir_query` (about a minute).

### E204 ir-step (number columns that step: a counter, a steady trend)
- Branch / parent: exp/204-ir-delta2 / E203 (stacked; the codec change sits on E200's `predict`)
- Question: after E200 the largest stream left in the drift reports is "earlier runs" (1,036 of 9,152 bytes under xz): a counter that is one more each time its link runs. `predict` only gets a value right when it repeats. Can a column also predict "last value beside this key, plus the step it took last time"?
- Change: `Options::step` (with `predict`): each number column keeps, for each of its line's first 4 cells, a map from that cell's value to the column's last number and step beside it, and how often the step repeated. A cell that is the best such map's last number plus its step (and is not already a repeat or a `predict` hit) is token 6, one byte. The decoder learns the same maps from the cells it decodes; flag bit 4 tells it to. Tests cover every option set (now 512), with counters that cross scales, signs, commas and percent signs.
- Result (xz -9 over the pack, against xz -9 over the files; best predict variant without / with step): reports-drift 0.543 / **0.505**; reports-random 0.654 / 0.643; exports 0.448 / **0.436**; tables 0.926 / 0.926. With row keys nothing changes (each row's stream already holds one counter, which xz sees). Decode of 300 drift reports with xz 28 ms to 37 ms. Blocks of 20 in the kernel with a keyframe and nearest 16: drift 0.734 / 0.713, random 0.767 / 0.763, exports 0.714 / 0.716.
- Verdict: **measured**. 7% smaller drift reports and 3% smaller exports for a third more decode time; best IR on drift is now 0.505 of xz. Off by default.
- Run: `cargo run --release --example ir_step` (about 5 minutes).

### E205 ir-query-predict (queries on the smallest IR: predict and step on)
- Branch / parent: exp/205-ir-query-predict / E204 (stacked)
- Question: E203's query refused `predict`, whose cells follow from other columns, so a queryable store cost 0.819 of xz on drift against 0.505 for the smallest IR. A predicted cell only follows from the first 4 cells of its own line, and those streams learn only from their own lines. Can the query decode just those columns, line by line, and keep the size?
- Change: `ir::query` takes `predict` and `step`: a read of slot k decodes slots 0 to min(k, 4) - 1 and k of that skeleton's lines, in file order, each with its stream's own state (and with row keys, only the rows asked for in `row`). Without `predict` and row keys it decodes the one stream straight through, as in E203. `siblings` and `nearest` are still refused (copied cells are in no stream). The test checks every column and one row of every option set the query takes against the files, on the edge-case sample and on runs where the predict and step tokens occur.
- Result (300 reports, IR parts in the kernel, each answer checked; stored against xz -9 over the files; the three E203 questions):

  | corpus, layout | stored | every status | one link's status | dev q under 0.05 | streams read |
  | --- | ---: | ---: | ---: | ---: | ---: |
  | drift, files in the kernel | 1.641 | 17.3 ms | 17.3 ms | 17.3 ms | all files |
  | drift, columns and numbers | 0.819 | 0.7 ms | 1.1 ms | 1.0 ms | 1 / 2 / 1 of 23 |
  | drift, + predict | 0.587 | 11.3 ms | 10.5 ms | 10.8 ms | 5 / 5 / 5 of 23 |
  | drift, + predict + step | **0.548** | 13.4 ms | 13.4 ms | 14.7 ms | 5 / 5 / 5 of 23 |
  | random, columns and numbers | 0.643 | 0.8 ms | 1.2 ms | 1.2 ms | 1 / 2 / 1 of 23 |
  | random, + predict + step | **0.619** | 13.2 ms | 13.4 ms | 15.1 ms | 5 / 5 / 5 of 23 |

  The smallest IR overall (predict, step, siblings, nearest; not queryable) is 0.505 of xz on drift and 0.643 on random: with predict and step, a queryable store costs 1.08 of it on drift and 0.96 on random. The query then reads 5 of 23 streams, and the time goes to the predictors learning on every cell (a full decode of all 300 reports takes 25 to 36 ms in memory with them, 13 ms without), not to the reads.
- Verdict: **measured**. A store can now be both nearly the smallest and queryable: 0.548 of xz with column queries in about 13 ms (1.3 times faster than reading the files), or 0.819 with queries in about 1 ms. The predictors' speed is the next thing to fix. Row-key layouts go through the line-by-line path and are slower than in E203 (2 to 3.5 ms); they lose on size anyway.
- Run: `cargo run --release --example ir_query` (about a minute).

### E206 ir-fast-predict (faster predictors, same bytes)
- Branch / parent: exp/206-ir-fast-predict / E205 (stacked)
- Question: with `predict` and `step`, decoding 300 drift reports took 36 ms against 13 ms without, and a column query 14 ms against 0.6 ms. Where does the time go, and how much comes back without touching the format?
- Change (no format change): a callgrind profile put a quarter of the time in malloc and free, then SipHash and `format!`. So: the value and step predictors share one map per earlier cell (one lookup instead of two); the predictor, dictionary and stream-index maps use a fixed FxHash; predictors take the line's cells by reference instead of a new vector per cell; numbers render and parse without `format!` or a digit vector; and the query opens its streams once instead of looking them up per cell. New `examples/ir_parity.rs` hashes the packs and blocks of 5 corpora under 32 option sets: identical before and after, so every stored IR decodes the same.
- Result (in memory, best of 25, 300 reports, before / after; this machine's times move by about 10% from run to run):

  | corpus | options | encode ms | decode ms | column query ms |
  | --- | --- | ---: | ---: | ---: |
  | drift | columns, numbers | 19.0 / 18.1 | 12.5 / 10.6 | 0.59 / 0.53 |
  | drift | + predict | 31.9 / 25.2 | 28.3 / 19.2 | 11.3 / **3.3** |
  | drift | + predict + step | 47.0 / 28.6 | 36.1 / **23.4** | 13.7 / **4.1** |
  | random | + predict + step | 50.9 / 39.8 | 33.7 / 26.0 | 10.8 / 4.1 |
  | exports | + predict + step | 19.4 / 17.3 | 17.6 / 11.8 | 2.6 / 0.9 |

  With the parts in the kernel (E205's layout), a column query on the 0.548-of-xz drift store now takes 4.0 ms (was 13.4), against 17.4 ms reading the files; on random reports 4.0 ms at 0.619 of xz.
- Verdict: **measured**. Same bytes; queries on the predicting IR 3 times faster, decode a third faster. Not an option: it changes no output.
- Run: `cargo run --release --example ir_speed` (about a minute), `cargo run --release --example ir_parity` (about 2 minutes; compare its output across commits).

### E207 ir-real (the IR on real files, not generated ones)
- Branch / parent: exp/207-ir-real / E206 (stacked)
- Question: E192 to E206 measured on generated reports, which were made to have the structure the IR looks for. How does it do on files made elsewhere?
- Change (measurement only): `examples/ir_real.rs` runs every IR variant, with and without `prose`, on two folders in the repo: the San Diego business tax registry fixture (2 CSV files, 51,000 rows, 10.8 MB) and the text files of the cases fixture (526 notes, playbooks, forms, scripts and logs of 18 sample businesses, 832 KB). Then the schema (E202) and a column query (E203, E205) on the registry. Only column names and counts are printed, no values.
- Result (xz -9 over the pack, against xz -9 over the files; every variant decoded byte-exact):

  | variant | registry (xz is 0.177 of the bytes) | cases text (0.156) | cases text, with prose |
  | --- | ---: | ---: | ---: |
  | templates | 1.041 | 1.304 | 1.081 |
  | columns | **0.966** | 1.360 | 1.105 |
  | numbers | 1.013 | 1.366 | 1.107 |
  | row keys | 1.271 | 1.442 | 1.138 |
  | predict | 1.009 | 1.367 | 1.107 |
  | predict, step | 1.009 | 1.367 | 1.107 |
  | + siblings, nearest 16 | 1.039 | 1.407 | 1.115 |

  By kind of file in the cases folder, the IR lost on every one (json 1.09, md 1.26, py 1.27, txt 1.31, log 1.46 with templates alone; columns made each worse). The schema found one dependency on the registry, ACTIVITY DESC from NAICS (held on 45,066 of 46,176 lines, 1,110 codes, no breaks), in 1.1 s. A column query on ownership type with predict and step took 105 ms against 72 ms splitting the files, because the predictors pull in the first columns of each row, which are near-unique ids and names.
- Verdict: **measured, and a caveat on everything before it**. On a real table the IR gains 3% over xz (columns only; numbers, predict and step add nothing, row keys cost 27%). On a real mixed folder it loses 8 to 37%, even with `prose`. The large gains of E192 to E206 hold for generated, repetitive reports; real data needs the IR to be chosen per file kind, never forced (E208).
- Run: `cargo run --release --example ir_real` (about 2 minutes).

### E208 ir-route (never lose to xz: each kind of file goes the smaller way)
- Branch / parent: exp/208-ir-route / E207 (stacked)
- Question: E207 found the IR loses to xz on a real mixed folder, by 8% even with `prose`, and that on a real table only one variant wins. Can the encoder find, per kind of file, whether the IR or the plain bytes compress smaller, so that turning the IR on never costs more than not having it?
- Change: `Options::route` (off by default). Files are grouped by extension and the skeleton of their first line, so reports from one template form one group and a folder's notes of the same extension another; groups of one file are pooled per extension. Each group is judged alone: xz -6 over its IR pack against its files stored plain (the raw part of E198), and goes the smaller way. The routed set is then judged as a whole against all IR and all plain, and the smallest of the three is kept. `ir::route` returns the choice. The decoder needs nothing new. The tests run every option set with `route` on the sample, and a spread of them in the property test.
- First pass, judged by zstd -9 with groups by extension only: zstd disagreed with xz -9 often enough to make the IR worse (drift reports with columns 0.927 to 0.998, because every md file, the reports included, went plain). That is why the judge is xz and the groups include the first-line template.
- Result (best variant, all with `prose`, xz -9 over the pack against xz -9 over the files; without / with `route`):

  | corpus | without | with | stored plain |
  | --- | ---: | ---: | --- |
  | reports-drift | 0.505 | 0.505 | none |
  | reports-random | 0.644 | 0.644 | none |
  | exports | 0.459 | 0.459 | none |
  | tables | 0.936 | 0.936 (0.995 for variants that lost) | csv, json, md where the IR lost |
  | drift + texts | 0.789 | 0.789 | none (prose already plain) |
  | cases text (real) | 1.105 | **0.995** | json, log, md, txt |
  | cases text + drift reports | 1.041 | **0.940** | json, log, the case md notes, txt |
  | registry (real) | 0.966 | 0.966 (1.000 for variants that lost) | csv where the IR lost |

  The worst result of any variant with `route` is 1.000 of xz; without it, 1.115. In the kernel in blocks of 20 with a keyframe, the cases text goes from 1.351 to 1.201 and tables from 1.169 to 0.980 (blocks pay their own restarts; the generated corpora are unchanged).
- Cost: encoding pays xz -6 over each group's IR and plain bytes, then up to three judgments of the whole set: 0.5 to 1.8 s for the 300 to 800 file corpora (12 to 57 ms without), and 13 s for the 10.8 MB registry (0.3 s without). That suits a background repack, not a live write.
- Verdict: **measured**. With `route`, the IR is never worse than xz over the files on any corpus tried, real or generated, and it keeps every gain it had. Its price is encode time. Off by default.
- Run: `cargo run --release --example ir_route [corpus ...]` (about 15 minutes, most of it the registry).

### E209 ir-tune (each column picks its own coding)
- Branch / parent: exp/209-ir-tune / E208 (stacked)
- Question: `numbers`, `predict` and `step` are on or off for every column at once. On the real registry (E207) number deltas cost 5% overall and predict 4%, though some columns surely gain. Can each column choose?
- Change: `Options::tune` (off by default). The encoder encodes once with each of: everything the options allow, no number deltas, no predicted cells, no stepped numbers, and their combinations; judges each stream's bytes with xz -6 under each; keeps per stream the smallest (by stream key: skeleton, slot, row key); and keeps the tuned pack only if xz judges the whole pack smaller than untuned. With `route`, the routing judges the IR as tuned, so a table the plain IR loses on can still win. The predictors still learn on every cell either way, so the decoder needs nothing new: each token says what it is. `ir::tune_modes` lists the choices. A unit test decodes every mode on every stream, mixed modes per stream, and blocks; the option tests run every option set with `tune`, and with `tune` and `route`.
- Result (best variant, all with `prose` and `route`, xz -9 over the pack against xz -9 over the files; without / with `tune`):

  | corpus | without | with | streams that changed coding |
  | --- | ---: | ---: | --- |
  | reports-drift | 0.505 | 0.504 | 4 of 23 |
  | reports-random | 0.644 | **0.633** | 5 of 23 |
  | exports | 0.459 | 0.458 | 4 of 35 |
  | tables | 0.936 | 0.936 | none |
  | cases text (real) | 0.995 | 0.995 | none (routed plain) |
  | cases text + drift | 0.940 | 0.940 | none for the best variant |
  | registry (real) | 1.000 (routed plain) | **0.961** | 15 of 41: 6 without number deltas, 9 without predicted cells, 10 without steps |

  With `numbers` alone, tuned: drift 0.767 to 0.749, random 0.670 to 0.654. The registry's 0.961 beats its best untuned variant (columns alone, 0.966). In the kernel in blocks of 20 with a keyframe, tuning on the whole set does not carry to blocks: random reports 0.712 to 0.722, the rest unchanged.
- Cost: up to 5 more encodes and an xz of every stream per try: 0.5 to 0.7 s for 300 reports (0.3 s with `route` alone), 5 s for the cases with drift reports, 47 s for the 10.8 MB registry (12 s with `route` alone).
- Verdict: **measured**. Small gains on generated reports (up to 1.7%), and on the real table it turns a loss into a 4% win. Expensive at encode; suits a background repack. Off by default.
- Side finding for the next entries: a plain transpose of the registry (each column's text one after another, no IR) is 0.953 of xz, better than any IR variant (0.961), so the IR's cell coding costs something on real tables; and sorting the rows by NAICS code, with the NAICS column kept in file order so the order needs no permutation stored, reaches 0.909 (Python prototype).
- Run: `cargo run --release --example ir_tune [corpus ...]` (about 10 minutes).

### E210 ir-literal (strings written out in full where xz matches them better)
- Branch / parent: exp/210-ir-literal / E209 (stacked)
- Question: a plain transpose of the real registry is 0.953 of xz, under the best IR (0.961). Does the IR lose it by writing repeated strings as indexes into each column's dictionary, where xz could match the strings themselves?
- Change: `Options::literal` (with `tune`, off by default): tuning also tries every stream with each string written out in full and no dictionary index. Because streams judged alone can mislead about the whole pack (a first run made random reports worse, 0.633 to 0.644), `tune` now judges the whole pack under no tuning, the picks without literal and the picks with it, and keeps the smallest; without `literal` it picks exactly as in E209. The decoder needs nothing new.
- Result (best variant, with `prose`, `route` and `tune`; without / with `literal`): drift 0.504 / 0.504, random 0.633 / 0.633, exports 0.458 / 0.458, tables 0.936 / 0.936, cases text 0.995 / 0.995, cases with drift 0.940 / 0.940, registry 0.961 / **0.960** (8 of 41 streams written out). With `numbers` and no `predict`, written-out strings help more (drift 0.749 to 0.700, exports 0.638 to 0.630), but those variants were never the best. Blocks unchanged. Encode about 1.5 to 1.7 times longer than `tune` alone (79 s on the registry).
- Verdict: **measured, no gain worth its cost**. The dictionary is not why the IR trails a plain transpose on the real table; the gap must come from the stream layout or token framing. Off by default; kept only because it costs nothing when off.
- Run: `cargo run --release --example ir_literal [corpus ...]` (about 20 minutes).

### E211 ir-sort (a table's rows stored in the order that compresses best)
- Branch / parent: exp/211-ir-sort / E210 (stacked)
- Question: a Python prototype put the real registry at 0.909 of xz when its rows are sorted by NAICS code and that column is kept in file order, so the order needs no permutation stored (0.905 sorted by NAICS then ZIP; a plain transpose without sorting is 0.953). How much of that does the IR keep?
- Change: `Options::sort` (off by default). Before the IR, each CSV file whose lines are its rows (an even number of quotes on every line, so no field spans lines) and that has at least 64 rows is rewritten as: header, the rows stably sorted by one column, then that column of every row in file order, one per line. The column is the one whose order judges smallest by xz -6 over the first 4,000 rows laid out by column, among columns neither constant nor near unique (distinct values under a quarter of the rows), and only if it beats the file's own order by 1%. The files part marks a rewritten file with its row count and whether it ended in a newline (flag bit 32, file kind bit 2); the decoder stably sorts the stored column to recover where each row goes. Column queries refuse a pack with sorted tables (their rows are in another order). `ir::sort_keys` reports the choice. A test covers quotes with commas, CRLF lines, short rows, empty lines, no final newline, and a field across lines (left alone), under a spread of option sets with `route`, `tune` and blocks.
- Result (xz -9 over the pack against xz -9 over the files, with `prose` and `route`; without / with `sort`):

  | variant | registry (real, 10.8 MB) |
  | --- | ---: |
  | columns | 0.966 / 0.947 |
  | numbers, tuned | 0.965 / 0.947 |
  | predict, step, tuned | 0.961 / **0.942** |

  Both registry files were sorted by NAICS, as in the prototype. The exports and tables fixtures have no CSV of 64 rows, so nothing changed there. Encode 51 s against 48 s (the key search adds about 3 s).
- Verdict: **measured**. The best real-table result yet: 0.942 of xz, a 6% gain where E207 started at 3%. The IR keeps about half of the prototype's gain over a plain transpose (0.909); the rest is lost in its cell framing, since the plain transpose of the unsorted table (0.953) also beats the IR's columns (0.966) on this table, and E210 ruled out the dictionary. Off by default.
- Next: store a sorted table's columns as plain text runs instead of IR tokens (a transpose path inside `route`), which the prototype says is worth another 3 to 4% here.
- Run: `cargo run --release --example ir_sort [corpus ...]` (about 5 minutes; the prototype is in the output file `ir_sort-output.md` of the night's notes).

### E212 ir-transpose (a table stored plain, laid out by column)
- Branch / parent: exp/212-ir-transpose / E211 (stacked)
- Question: E211 left the registry at 0.942 of xz; the same sort over a plain transpose (Python) was at 0.909. Does storing a sorted table plain, laid out by column, close that gap?
- Change: `Options::transpose` (off by default). A CSV table (the same lines `sort` accepts: no field across lines, 64 rows or more) that `route` or `prose` stores as it is goes into the raw part laid out by column: the header, each line's field count, then each column's fields one per line. The files part marks it with its line count and whether it ended in a newline (flag bit 64, file kind bit 4); the decoder joins each line's fields with commas, then unsorts it if `sort` rewrote it. `route` judges the IR against this layout, so it picks it only where it is smaller. Raw files are not in column queries, so a table stored this way cannot be queried by column (the query index skips it). Tests cover the E211 tables under a spread of option sets with `route` or `prose`, with and without `sort`, and blocks.
- Result (registry, real, 10.8 MB; xz -9 over the pack against xz -9 over the files, with `prose` and `route`):

  | variant | route | + sort | + transpose | + both |
  | --- | ---: | ---: | ---: | ---: |
  | columns | 0.966 | 0.947 | 0.965 | **0.930** |
  | predict, step, tuned | 0.961 | 0.942 | 0.961 | **0.930** |

  With both, `route` stores both registry files plain by column (11.5 MB raw part); with `transpose` alone it keeps the IR. The generated corpora have no CSV of 64 rows, so nothing changed there. Encode time is unchanged (14 s columns, 48 s tuned).
- A Python check of where the rest of the prototype's gap goes: moving the stored key column, dropping the field counts, or leaving the key out of the sorted rows each change 0.000 to 0.003. The prototype parsed the CSV and so dropped 1.44 million quote characters, which is not exact; with quotes kept its layout is 0.927. Leaving out the quotes of a column whose every field is quoted (13 of the registry's 14) gives 0.913.
- Verdict: **measured**. 0.942 to 0.930, within 0.003 of the exact bound for this layout, and with no tuning needed (0.930 with columns alone, at a third of the encode time). Off by default.
- Next: E213, a column whose fields are all quoted stored without its quotes.
- Run: `cargo run --release --example ir_transpose [corpus ...]` (about 7 minutes on the registry).

### E213 ir-quotes (a column quoted in every row stored without its quotes)
- Branch / parent: exp/213-ir-quotes / E212 (stacked)
- Question: 13 of the registry's 14 columns quote every field. A Python check put the E212 layout at 0.913 with those quotes left out. Does the kernel get it?
- Change: no new option; it changes the `transpose` layout (E212, not merged). After the header, one byte per column: `q` where every field of the column starts and ends with a quote (those two are left out of its fields), `-` elsewhere. The decoder puts them back. A field such as `""` or one with inner quotes comes back exactly, since only the outer two are dropped. A unit test covers empty quoted fields, inner quotes, a short row and a longer one; the table test gains a table with columns quoted in every row, in most rows, and never.
- Result (xz -9 over the pack against xz -9 over the files, with `prose` and `route`):

  | variant | route | + sort | + transpose | + both |
  | --- | ---: | ---: | ---: | ---: |
  | columns | 0.966 | 0.947 | 0.954 | **0.911** |
  | predict, step, tuned | 0.961 | 0.942 | 0.954 | **0.911** |

  The raw part shrinks from 11.5 to 10.1 MB. `transpose` alone, unsorted, now beats the IR on this table too (0.954). Exports, tables and the cases folder are unchanged. Encode 14 s with columns.
- Verdict: **measured**. The real registry at 0.911 of xz, from 0.966 at E207: 9% smaller, exactly, with the plain-columns variant and no tuning. Off by default, with `transpose`.
- Next: the same columns layout for JSON arrays of records, which the real fixtures do not have yet; and `transpose` inside blocks for large tables, so one range of rows reads without the whole table.
- Run: `cargo run --release --example ir_transpose [corpus ...]` (about 8 minutes).

### E214 ir-sample (route and tune judged on a sample)
- Branch / parent: exp/214-ir-sample / E213 (stacked)
- Question: `route` and `tune` judge with xz over whole encodes: 13 s (columns) and 43 s (tuned) on the 10.8 MB registry, against well under a second to encode it. Do they choose the same on a sample?
- Change: `Options::sample` (bytes, 0 is off). `route` and `tune` judge on every file, each file over its share of the budget (and over 4 KB) cut to its first line and an even spread of its other lines; what they choose applies to the whole set. Only the encoder changes. A first try also kept only every `r`th file of a large set: it misjudged, routing the drift reports plain (0.926 to 0.998 with columns) and the exports at 256 KB, since the IR's gain is across neighbouring files; so every file is kept.
- Result (with `prose`, `route`, `sort` and `transpose`; xz -9 over the pack against xz -9 over the files; judging on everything / 1 MB / 256 KB):

  | corpus | columns | encode ms | predict, step, tuned | encode ms |
  | --- | --- | --- | --- | --- |
  | reports-drift | 0.926 / 0.926 / 0.926 | 292 / 167 / 172 | 0.504 / 0.504 / 0.504 | 553 / 343 / 312 |
  | reports-random | 0.717 / 0.717 / 0.717 | 338 / 198 / 195 | 0.630 / 0.630 / 0.630 | 634 / 494 / 440 |
  | exports | 0.662 / 0.662 / 0.662 | 261 / 255 / 219 | 0.617 / 0.617 / 0.617 | 532 / 533 / 554 |
  | cases text (real) | 0.996 / 0.996 / 0.996 | 1327 / 1271 / 725 | 0.995 / 0.995 / 0.995 | 4501 / 4371 / 3042 |
  | cases text + drift | 0.989 / 0.989 / 0.989 | 1508 / 1107 / 1052 | 0.940 / 0.940 / 0.940 | 5518 / 4040 / 3620 |
  | registry (real) | 0.911 / 0.911 / 0.911 | 13089 / 4999 / 4414 | 0.911 / 0.911 / 0.911 | 43383 / 7109 / 4583 |

  Every size is the same as judging on everything, and every route choice too (same raw part). The drift and random times also fell where no file was cut; part of that is run order (the first encode of each row runs cold), so only the registry's are clear gains. About 3 s of the registry's remaining time is `sort` choosing its key.
- Verdict: **measured**. 1 MB keeps every result and makes the registry's tuned encode 6x faster (43 to 7 s). Off by default; 1 MB is the value to try first.
- Run: `cargo run --release --example ir_sample [corpus ...]` (about 6 minutes).

### E215 ir-preset (one setting for every corpus)
- Branch / parent: exp/215-ir-preset / E214 (stacked)
- Question: each option so far was measured where it helps. Is one setting close to the best on every corpus, generated and real, and never worse than xz? That is what a default would need.
- Change: none to the codec; new `examples/ir_preset.rs`. Every setting has `prose`, `route`, `sort`, `transpose` and `sample` at 1 MB.
- Result (xz -9 over the pack against xz -9 over the files):

  | corpus | safe (columns) | full | full, no row keys | full, tuned | predict, step | predict, step, tuned |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: |
  | reports-drift | 0.926 | 0.593 | 0.539 | 0.593 | 0.505 | **0.504** |
  | reports-random | 0.717 | 0.741 | 0.683 | 0.741 | 0.645 | **0.630** |
  | exports | 0.662 | 0.528 | **0.459** | 0.528 | 0.617 | 0.617 |
  | tables | 0.995 | 0.995 | **0.936** | 0.995 | 0.995 | 0.995 |
  | texts | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 |
  | drift + texts | 0.969 | 0.827 | 0.802 | 0.829 | **0.789** | **0.789** |
  | cases text (real) | 0.996 | 1.002 | 0.996 | 1.002 | **0.995** | **0.995** |
  | cases text + drift | 0.989 | 0.956 | 0.944 | 0.955 | **0.940** | **0.940** |
  | registry (real) | **0.911** | **0.911** | **0.911** | **0.911** | **0.911** | **0.911** |

  Full is columns, numbers, siblings, row keys, nearest 16, predict and step. Encode times: under 1.6 s on every generated corpus, 1.1 to 4.8 s on the cases folder, 5 to 14 s on the registry (7.3 s for predict, step, tuned). Decode under 35 ms except the registry (160 to 220 ms).
- Verdict: **measured**. Two settings split the corpora. "Predict, step, tuned" is best or tied on 7 of 9 and never worse than xz; it loses only where one file repeats another's cells (exports 0.617 against 0.459, tables 0.995 against 0.936), which `siblings` and `nearest` catch and which cost the reports 3 to 8%. Row keys make every corpus worse or no better once `route` and `predict` are on. No one setting is best everywhere yet.
- Next: E216, `route` also judging per kind of file whether its files diff against a base file, so one setting can take both sides.
- Run: `cargo run --release --example ir_preset [corpus ...]` (about 3 minutes).

### E216 ir-bases (route also judges whether files diff against a base)
- Branch / parent: exp/216-ir-bases / E215 (stacked)
- Question: E215 found two settings split the corpora: `siblings` and `nearest` win where one file repeats another's cells (exports, tables) and cost the reports 3 to 8%, whose columns the predictors already catch. Can `route` take each side where it wins?
- Change: `Options::bases` (off by default; with `route` and `siblings` or `nearest`). For each kind of file, `route` now judges three ways: through the IR, as it is, and through the IR with no base file (every cell stored, predicted and stepped as usual). The files part already says per file whether it has a base, so the decoder needs nothing new. `ir::unbased` reports the choice. The option sets in the tests take `bases` in three of four.
- Result (all with `prose`, `route`, `sort`, `transpose`, `sample` at 1 MB, and no row keys; xz -9 over the pack against xz -9 over the files; encode time):

  | corpus | predict, step, tuned (E215) | + siblings, nearest 16 | + siblings, nearest, bases | same, untuned |
  | --- | ---: | ---: | ---: | ---: |
  | reports-drift | 0.504 (0.3 s) | 0.537 | **0.504** (0.7 s) | 0.505 (0.3 s) |
  | reports-random | 0.630 (0.4 s) | 0.673 | **0.630** (1.3 s) | 0.645 (0.4 s) |
  | exports | 0.617 | 0.458 | **0.458** (1.0 s) | 0.459 (0.3 s) |
  | tables | 0.995 | 0.936 | **0.936** | 0.936 |
  | texts | 1.000 | 1.000 | 1.000 | 1.000 |
  | drift + texts | 0.789 | 0.802 | **0.789** (1.6 s) | 0.789 (0.6 s) |
  | cases text (real) | 0.995 (4.1 s) | 0.996 | **0.995** (6.1 s) | 0.995 (1.4 s) |
  | cases text + drift | 0.940 | 0.944 | **0.940** (5.9 s) | 0.940 (1.7 s) |
  | registry (real) | 0.911 (7.2 s) | 0.911 | **0.911** (10.2 s) | 0.911 (5.5 s) |

  Never worse than xz on any corpus. Decode 4 to 31 ms on the generated and cases corpora, 173 ms on the registry.
- Verdict: **measured**. With `bases`, one setting is best or tied on all nine corpora: columns, numbers, siblings, nearest 16, predict, step, tune, prose, route, sort, transpose, bases and a 1 MB sample. Untuned, it gives up at most 0.015 (reports-random) for 2 to 4x faster encodes. This is the setting to make the default if the IR is ever turned on; every option stays off by default here.
- Run: `cargo run --release --example ir_bases [corpus ...]` (about 4 minutes).

### E217 ir-store (the E216 setting in ikam's store)
- Branch / parent: exp/217-ir-store / E216 (stacked)
- Question: E216's setting was measured as one pack under xz. What does ikam's kernel store with it, against storing the files themselves, whole and in blocks a read can open alone?
- Change: none to the codec; new `examples/ir_store.rs`. The example helper that turns a pack into kernel entries now includes the raw part (it had no caller with one, so no earlier number changes).
- Result (untuned; tuned is within 0.007 everywhere). Each cell is against the files in the kernel, then against xz -9 over the files; one file read from the middle in memory:

  | corpus | files | files in the kernel (of xz) | pack, one entry | blocks of 20 (read) | blocks of 50 (read) |
  | --- | ---: | ---: | ---: | ---: | ---: |
  | reports-drift | 300 | 1.641 | **0.303** / 0.498 | 0.432 / 0.709 (2.1 ms) | 0.381 / 0.625 (5.3 ms) |
  | reports-random | 300 | 1.315 | **0.502** / 0.660 | 0.541 / 0.711 (2.4 ms) | 0.523 / 0.688 (9.8 ms) |
  | exports | 300 | 2.006 | **0.228** / 0.458 | 0.358 / 0.717 (0.8 ms) | 0.277 / 0.557 (1.3 ms) |
  | tables | 72 | 1.300 | **0.700** / 0.910 | 0.775 / 1.007 (0.3 ms) | 0.738 / 0.959 (0.3 ms) |
  | texts | 24 | 1.104 | **0.849** / 0.937 | 0.863 / 0.953 | 0.851 / 0.939 |
  | drift + texts | 324 | 1.354 | **0.560** / 0.759 | 0.635 / 0.859 (2.0 ms) | 0.597 / 0.808 (5.3 ms) |
  | cases text (real) | 526 | 1.441 | **0.672** / 0.968 | 0.833 / 1.201 (0.5 ms) | 0.819 / 1.180 (0.9 ms) |
  | cases text + drift | 826 | 1.503 | **0.611** / 0.919 | 0.781 / 1.175 (0.5 ms) | 0.749 / 1.126 (0.9 ms) |
  | registry (real) | 2 | 1.005 | **0.916** / 0.920 | 0.916 / 0.920 (54 ms) | same |

- Verdict: **measured**. In the store, the setting is smaller than the files on every corpus: a third of the bytes for drift reports, under a quarter for exports, two thirds for the real cases folder, 0.916 for the real registry. Blocks keep most of that with 1 to 10 ms reads; for prose-heavy folders (cases) blocks lose a fifth of the gain, since each block's raw part restarts its compression. A large table is one file, so its blocks are the whole table (54 ms); row ranges would need a table cut into blocks of rows.
- Next: blocks that share one raw part's dictionary across blocks (as the keyframe does for streams), and row-range blocks for large tables.
- Run: `cargo run --release --example ir_store [corpus ...]` (about 4 minutes).

### E218 ir-damage (a damaged pack never crashes the decoder)
- Branch / parent: exp/218-ir-damage / E217 (stacked)
- Question: E208 to E216 added new layouts (plain files, sorted and transposed tables, quoted columns, unbased files, tuned streams). Before any becomes a default: does a damaged pack or block ever panic the decoder instead of returning an error?
- Change: two tests. `damaged_packs_never_panic` encodes the sample and big tables under four settings (E216's, without route, with row keys, group and literal, and plain), then decodes 3,000 changed packs per setting (a byte set, a byte dropped, the pack cut short) and 1,000 blocks with one part changed, in release and debug builds. `huge_table_counts_are_refused` hands the decoder a sorted, transposed table whose row, line and length counts are huge. Fixes: the sorted-table check `1 + 2 * rows` (E211) overflowed on a huge count; in a release build it wrapped, passed, and then sliced past the end, a crash. It is now a checked multiply. Number deltas and steps (E192, E204) now add and subtract wrapping, so a damaged delta cannot overflow (valid numbers have at most 30 digits, so no valid pack changes).
- Result: before the fix, the huge-count test panicked (debug and release). After it, both tests pass in both builds; `ir_parity` hashes are identical to E206's (160 packs and blocks), so every stored pack decodes the same.
- Side finding (no entry): Word, Excel and PowerPoint files unzipped and rezipped exactly (zlib level 6 reproduced every member of the cases fixtures) store at 0.17 to 0.54 of xz over the files, but the kernel has done this since E080 and E083 (`container.rs`), so files in the kernel already have it.
- Verdict: **kept** (a fix and tests). Merge the stack from its top so the E211 fix comes with it.

### E219 ir-scale (the E216 setting as a set of reports grows)
- Branch / parent: exp/219-ir-scale / E218 (stacked)
- Question: `sample` (E214) cuts only large files, so many small files are judged whole by `route` and `tune`. How do size and encode time grow with the number of reports, and can the sample cover many small files without misjudging (E214's first try, keeping every `r`th file, routed the drift reports plain)?
- Change: when the files cut by `sample` are still over twice the budget, `route` and `tune` judge four runs of neighbouring files spread over the set, about a quarter of the budget each, so neighbours stay together. New `examples/ir_scale.rs` (drift and random reports at 300, 1,000 and 3,000 files).
- Result (E216 setting, sample 1 MB; xz -9 over the pack against xz -9 over the files; encode time):

  | reports | files | MB | untuned before | untuned with runs | tuned before | tuned with runs |
  | --- | ---: | ---: | --- | --- | --- | --- |
  | drift | 300 | 1.3 | 0.505 (0.3 s) | 0.505 (0.2 s) | 0.504 (0.7 s) | 0.504 (0.7 s) |
  | drift | 1,000 | 5.6 | 0.359 (0.7 s) | 0.359 (0.5 s) | 0.356 (2.4 s) | 0.359 (0.9 s) |
  | drift | 3,000 | 18.2 | 0.311 (2.4 s) | 0.311 (0.8 s) | 0.305 (7.1 s) | 0.311 (1.3 s) |
  | random | 300 | 1.3 | 0.645 (0.4 s) | 0.645 (0.3 s) | 0.630 (1.4 s) | 0.630 (1.4 s) |
  | random | 1,000 | 5.6 | 0.615 (1.4 s) | 0.615 (0.8 s) | 0.593 (4.9 s) | 0.599 (2.4 s) |
  | random | 3,000 | 18.2 | 0.605 (4.7 s) | 0.605 (1.1 s) | 0.572 (16.3 s) | 0.586 (2.6 s) |

  Untuned, every size is the same and encodes are 1.4 to 4x faster; tuned, they are 2 to 6x faster and up to 0.014 larger, since `tune` picks each column's coding from fewer rows. The nine E216 corpora give the same sizes as E216. Decode grows linearly (about 20 ms per MB). The ratio itself improves as the set grows: drift reports at 3,000 files are 0.31 of xz.
- Verdict: **measured**. The setting scales linearly; with runs, 18 MB of reports encode in about a second untuned. Off by default (part of `sample`).
- Run: `cargo run --release --example ir_scale [files ...]` (about 2 minutes).

### E220 ir-row-groups (a large table cut into row groups)
- Branch / parent: exp/220-ir-row-groups / E219 (stacked)
- Question: E217 left a large table as one block (the whole registry file, 54 ms to read). If a table were cut into groups of rows, each its own block, how much of E213's gain survives, and how fast is one group to read? A keyframe (E197) or a shared dictionary would let each group go on from the first.
- Change: none to the kernel. Prototype `packages/kernel/scripts/ir_row_groups.py` lays each group out as E213 does (sorted by NAICS with the key in file order, columns one after another, outer quotes of always-quoted columns left out) and compresses it with xz -9, alone or after group 0 (estimated as xz(group 0 + group) minus xz(group 0)).
- Result (real registry, two files of about 25,600 rows; against xz -9 over the files):

  | rows per group | groups | independent | after group 0 | read one group |
  | ---: | ---: | ---: | ---: | ---: |
  | whole tables | 2 | 0.912 (solid) | | |
  | 30,000 (one per file) | 2 | 0.925 | 0.925 | 57 ms |
  | 10,000 | 6 | 0.981 | 0.955 | 43 ms |
  | 3,000 | 18 | 1.065 | 1.017 | 15 ms |
  | 1,000 | 52 | 1.183 | 1.106 | 5 ms |

  Python's xz decodes the two whole files in 147 ms.
- Verdict: **measured, not built**. Groups lose the gain fast: at 10,000 rows half of it is gone, and below about 4,000 rows the groups are larger than xz over the plain files, even after group 0. Reads get faster only in proportion to the group size. If row ranges are needed, 10,000-row groups after a keyframe are the knee (0.955, about a third of the whole-table read).
- Run: `python3 -I packages/kernel/scripts/ir_row_groups.py tests/fixtures/san-diego-business-registry/*.csv` (about a minute).

### E221 ir-block-raw (each block's raw text as its own store entry)
- Branch / parent: exp/221-ir-block-raw / E220 (stacked)
- Question: E217's blocks lost about a fifth of the whole pack's gain on prose-heavy folders (cases: 0.672 whole, 0.819 in blocks of 50). Each block's raw part (E198, files kept as they are) was packed with its streams in one entry. If each raw part were its own entry, would the kernel's repack (templates, trained dictionaries) win some of that back by seeing the raw parts as similar entries?
- Change: none to the kernel. New `examples/ir_blockraw.rs` stores E216's setting (untuned) in blocks of 20 and 50 with a keyframe, with each block's raw part packed with its streams (as E217) or as its own entry.
- Result (against the files in the kernel):

  | corpus | whole pack | blocks of 20: together / raw apart | blocks of 50: together / raw apart |
  | --- | ---: | ---: | ---: |
  | reports-drift | 0.303 | 0.432 / 0.432 | 0.381 / 0.381 |
  | exports | 0.228 | 0.358 / 0.358 | 0.277 / 0.277 |
  | tables | 0.700 | 0.775 / 0.775 | 0.738 / 0.738 |
  | texts | 0.849 | 0.863 / 0.912 | 0.851 / 0.851 |
  | drift + texts | 0.560 | 0.635 / 0.657 | 0.597 / 0.600 |
  | cases text (real) | 0.672 | 0.833 / 0.821 | 0.819 / 0.811 |
  | cases text + drift | 0.611 | 0.781 / 0.798 | 0.749 / 0.760 |

- Verdict: **dropped**. Raw parts apart win at most 0.012 (cases alone) and lose up to 0.049 elsewhere; the repack does not recover what a block's raw text loses by being compressed apart from the rest of the folder. Blocks of 50 stay the better block size. What would help is a raw part that later blocks compress against (as the keyframe does for streams), which needs a dictionary in the block format, not a different layout in the store.
- Run: `cargo run --release --example ir_blockraw [corpus ...]` (about a minute).

### E222 ir-north (the E216 setting on the north corpora)
- Branch / parent: exp/222-ir-north / E221 (stacked)
- Question: before the IR goes behind a put or repack option, what would it do to the north score (E099), whose three corpora mix text files with office and pdf files that share their facts? The kernel already shares text across formats (E112 and earlier), and an IR pack takes the text files out of that sharing.
- Change: none to the kernel. New `examples/ir_north.rs` stores each north corpus (the same files as `bench north`) in the kernel as it is, and with its text files (md, html, json, jsonl, csv, log, py) replaced by one IR pack of them under E216's setting (untuned), office and pdf files left to the kernel; and the text files alone, in the kernel and as the pack in the kernel.
- Result (bytes stored; the kernel over every file matches the bench's `kernel` field for each corpus):

  | corpus | files (text) | kernel, every file | text as one IR pack | text alone: kernel / IR pack in the kernel |
  | --- | ---: | ---: | ---: | ---: |
  | north/cases | 792 (510) | 391,496 | 319,992 (0.817) | 190,312 / 125,339 (0.659) |
  | north/multiformat | 96 (48) | 332,403 | 321,975 (0.969) | 45,619 / 33,457 (0.733) |
  | north/tables | 144 (72) | 135,551 | 162,554 (1.199) | 6,497 / 4,636 (0.714) |

  Geometric mean: 0.983 for the whole corpora (north score 0.3035 would become about 0.298), 0.701 for the text files alone. Encode 1.5 s at most.
- Verdict: **measured**. On text files alone the IR stores 0.66 to 0.73 of what the kernel stores. In a mixed corpus it wins where the text stands on its own (cases 0.817) and loses where the text repeats an office or pdf file's facts (tables 1.199: the kernel's sharing of the same records across csv, json, md, xlsx, docx and pdf saves more than the IR's coding of the csv, json and md alone). So an IR option in repack needs a judge at the store, as `route` (E208) is in the pack: keep the IR pack only where the store is smaller with it. With that judge the three corpora would be 0.817, 0.969 and 1.000, a geometric mean of 0.925 (north about 0.281), at the cost of a second repack of the text files. (E223: with the files kept readable by id, this does not hold; judged 0.993.)
- Next: an `ir` repack option that encodes a tree's text files with the E216 setting and keeps the pack only when the stored bytes go down; then the bench's north score with it on.
- Run: `cargo run --release --example ir_north` (about 1.5 minutes).

### E223 ir-repack (files kept readable as members of an IR pack in the store)
- Branch / parent: exp/223-ir-repack / E222 (stacked)
- Question: E222 replaced a corpus's text files with one IR pack, so those files could no longer be read by their ids. Kept addressable, as members of the pack in the store, how much of that gain is left, and would a store-level judge (keep whichever is smaller) move the north score?
- Change: two builtins, `ir-member/1` (`ir_member(pack, index)`) and `ir-block-member/1` (`ir_block_member(skeletons, block, index)`, blocks without a keyframe), appended to the builtin table, and `Repo::put_ir` and `Repo::put_ir_blocks` (`src/ir/store.rs`): the pack (or each block) is content, each file the fact naming it, so repack keeps the pack and drops the files' own bytes. The last pack or block decoded on each thread is kept, so reading a pack's files in turn decodes it once. Nothing calls these unless asked; put and repack are unchanged. Test `ir_members_in_the_store` (whole and blocks, read back after repack, fsck clean; an index past the pack is refused). New `examples/ir_repack.rs` stores each corpus with its text files (md, html, json, jsonl, csv, log, py) as themselves, as one IR pack's members, or as blocks of 50 or 20, every file read back by id after repack and fsck clean.
- Result (bytes after repack against the files with short records; E104's `short_records`, which IR members need; read time of one text file):

  | corpus | files (text) | files | short records: files | IR pack (read ms first / next) | blocks of 50 (read ms) | blocks of 20 (read ms) | judged |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
  | north/cases | 792 (510) | 391,496 | 361,242 | 0.982 (28.8 / 1.2) | 1.113 (0.1) | 1.096 (0.3) | 0.982 |
  | north/multiformat | 96 (48) | 332,403 | 310,415 | 0.999 (0.7 / 0.5) | 0.997 (0.5) | 1.031 (0.5) | 0.997 |
  | north/tables | 144 (72) | 135,551 | 122,678 | 1.016 (0.9 / 0.2) | 1.039 (0.7) | 1.039 (0.6) | 1.000 |
  | reports-drift | 300 (300) | 27,657 | 27,657 | 1.072 (19.0 / 0.2) | 1.394 (4.1) | 1.397 (2.5) | 1.000 |
  | exports | 300 (300) | 22,036 | 22,036 | 1.193 (3.5 / 0.4) | 1.493 (0.9) | 1.795 (0.8) | 1.000 |
  | cases text | 549 (549) | 216,843 | 210,521 | 1.124 (34.7 / 1.7) | 1.298 (2.3) | 1.267 (1.1) | 1.000 |

  North corpora (geometric mean against the files with plain records): short records alone 0.920; judged with IR members 0.914, so the IR adds 0.007 on top of short records.
- Why so little, when the pack alone stores 0.66 to 0.73 of the text (E222): every member costs one derivation record holding its file's 32-byte id (cases text: 549 records, 30 KB short, 74 KB plain, against a 125 KB pack), and the kernel's files were already small (92 bytes a report after templates and the dictionary). And the planner stores the pack with zstd, not brotli (cases text 195 KB against 144 KB alone), because it charges a decode of the pack for each member read; teaching its read model about the member cache (first try, reverted) did not change the codec, since repack also plans some text files as slices of the pack, and those count the pack's reads again. Blocks make reads fast (0.1 to 4 ms) but cost more records per file and lose the cross-block bases.
- Verdict: **measured, not worth turning on**. The members work and stay exact, but at rest they do not beat the kernel's own files except by about 2% on the fixture cases. E222's forecast of north 0.281 assumed files no one reads by id; it does not hold for files kept addressable. The IR's size gains belong where whole folders are written and read together (exports, archives, E217's blocks as entries), not as a per-file storage plan.
- Run: `cargo run --release --example ir_repack [corpus ...]` (about 7 minutes).

### E224 ir-raw-keyframe (raw text of blocks compressed after the keyframe)
- Branch / parent: exp/224-ir-raw-keyframe / E223 (stacked)
- Question: E221 found that storing blocks' raw parts apart does not win back what prose-heavy folders lose in blocks. How much would it win if each block's raw text were compressed after the keyframe block's (block 0's) raw text, as the keyframe already does for streams?
- Change: none to the kernel. Prototype `packages/kernel/scripts/ir_raw_keyframe.py`: the fixture cases' text files (the raw part, since `route` keeps them plain) in blocks, each block compressed with xz -9 alone, after block 0 (estimated as xz(block 0 + block) minus xz(block 0)), or after a sample of every block's first file.
- Result (526 files, 832 KB; against xz -9 over every file at once, 128,968 B):

  | blocks of | blocks | alone | after block 0 | after a sample |
  | ---: | ---: | ---: | ---: | ---: |
  | 20 | 27 | 1.830 | 1.725 | 1.455 |
  | 50 | 11 | 1.531 | 1.222 | 1.375 |
  | 100 | 6 | 1.334 | 1.104 | 1.306 |

- Verdict: **measured, worth building**. Compressing a block's raw text after block 0's cuts blocks of 50 by a fifth (1.531 to 1.222 of the whole), and a reader of any block already decodes block 0 for the keyframe, so the read cost is small. The kernel's related-content frames (E057) could in principle do this, but E221 shows they do not fire for raw parts stored apart. Building it needs the block format to name block 0's raw part as the frame's dictionary (zstd with a raw-content dictionary), or the planner to pick it as the related base.
- Next: raw parts of later blocks as zstd frames with block 0's raw part as dictionary, in `encode_blocks` with `keyframe`; measure E217's cases blocks again (now 0.819 against 0.672 whole).
- Run: `python3 -I packages/kernel/scripts/ir_raw_keyframe.py tests/fixtures/cases` (about a minute).

### E225 ir-raw-dict (later blocks' raw text compressed against the keyframe's, in the store)
- Branch / parent: exp/225-ir-raw-dict / E224 (stacked)
- Question: E224's prototype (under xz) found a block's raw text a fifth smaller after block 0's. Does that hold in ikam's store, which compresses each block entry with its own codecs and a trained dictionary?
- Change: `Options::raw_keyframe` (off by default; blocks with a keyframe only): `encode_blocks` stores each later block's raw part as a zstd -19 frame with the first block's raw part as its dictionary, where that is smaller than the raw part under xz -6 (as `route` judges); `Blocks::raw_framed` marks those blocks, and `Reader` unframes them before decoding (a frame over 4 GiB is refused). `tests/ir.rs` turns it on in half of its option sets. New `examples/ir_rawkey.rs` (E217's blocks in the kernel, without and with it).
- Result (each block one entry in the kernel; against the files in the kernel; read ms of the middle block):

  | corpus | whole pack | blocks of 20: before / raw keyframe | blocks of 50: before / raw keyframe |
  | --- | ---: | ---: | ---: |
  | reports-drift | 0.303 | 0.432 / 0.432 | 0.381 / 0.381 |
  | exports | 0.228 | 0.358 / 0.358 | 0.277 / 0.277 |
  | tables | 0.700 | 0.775 / 0.775 | 0.738 / 0.738 |
  | texts | 0.849 | 0.863 / 0.864 | 0.851 / 0.851 |
  | drift + texts | 0.560 | 0.635 / 0.635 | 0.597 / 0.597 |
  | cases text (real) | 0.672 | 0.833 / 1.249 | 0.819 / 0.869 |
  | cases text + drift | 0.611 | 0.781 / 1.126 | 0.749 / 0.785 |

  Reads are unchanged (0.0 to 7 ms). Judging the frame against the raw part as it is (first run) instead of under xz -6 gave the same cases figures.
- Verdict: **dropped** (the option stays, off). In the store, a framed raw part is worse wherever it is used: the kernel already compresses a block entry better (brotli or xz, its trained dictionary across entries) than zstd does with block 0 as dictionary, and a frame cannot be compressed again. E224's gain is real under one compressor but the store gets most of it already. A gain here would need the store itself to compress a block against block 0 (E057's related-content frames), not the IR.
- Run: `cargo run --release --example ir_rawkey [corpus ...]` (about a minute).

### Merge of E197 to E225 into main (Nov 11)
- On Eric's word ("Merge it"), the stacked branches exp/197-ir-blocks to exp/225-ir-raw-dict merged onto main a831e15 as one squashed commit, 1b72f35 (its tree is exp/225's tree). The build files committed by mistake on exp/206 are not in main's history. Every IR option is off, and nothing in put, repack or the bench calls `ir`, `put_ir` or `put_ir_blocks`.
- Gates on the merge: every kernel test suite passes (release); clippy `-D warnings` and fmt clean; the bench gives the same byte and count fields as main's last run (only times and peak memory differ) and north 0.3035; `ir_parity` hashes are identical to E206's.

### E226 commit-cost (where a report copy's time goes, as `ikam live` stores it)
- Branch / parent: exp/226-commit-cost / main 1c5efb9
- Question: the Mercante report-copy demo on a Mac (Nov 10) reported 47 ms a copy, with `store_ms` 4.7 and `tree_ms` 16.7 (`X-Ikam-Cost`), and E178 found a commit that drops a superseded copy reads the whole store (2.2 s at 1M). Where does a copy's time go: tree CPU, store reads, or writes?
- Change: none in the kernel. `examples/commits.rs`: as `ikam live` does for one PUT, `put_content` of a lead-lag report (E211's `reports` generator), `edit_tree` at `reports/<name>`, then `Repo::commit`, on one handle reopened after setup, with N reports already in the folder; 20 copies after one warm-up, medians; a counting store wrapper reports writes, reads, whole-store scans and drops. FsStore in a temporary directory (ext4, this Linux box) and MemStore (CPU only).
- Result (two runs; FsStore times vary about 2x between runs on this box, MemStore little):

  | reports in folder | MemStore copy ms (tree ms) | FsStore copy ms (tree ms) | commit ms (FsStore) | first copy on a fresh handle ms (reads) |
  | ---: | ---: | ---: | ---: | ---: |
  | 2 | 0.93 / 0.97 (0.52 / 0.51) | 4.43 / 12.31 (2.90 / 8.49) | 1.00 / 2.68 | 3.3 / 10.5 (28) |
  | 30 | 1.12 / 0.96 (0.64 / 0.51) | 5.07 / 11.03 (3.39 / 7.62) | 1.03 / 2.68 | 9.1 / 16.0 (162) |
  | 300 | 2.30 / 1.74 (1.44 / 1.02) | 5.99 / 11.23 (4.00 / 7.73) | 0.92 / 2.34 | 43.3 / 71.1 (2,059) |
  | 3000 | 9.89 / 8.10 (8.26 / 6.75) | 12.09 / 13.79 (9.06 / 10.29) | 1.02 / 1.17 | 355.5 / 348.0 (20,649) |

  - Writes a copy, at every size: the report 2 (content and its derivation record), the trees 5 (the folder's tree as a slice of its last version plus its pointer, the root's tree plus its pointer), the commit 2 (the commit and the ref). Bytes stored a copy: report 526 to 1,420, trees 464 to 503, commit 86. FsStore syncs each of the 9.
  - `edit_tree` already writes only the trees on the changed path; what grows with the folder is CPU in planning its one big tree (at 3,000 files a 157 KB encoding: template search about 3 ms, slice search 0.8, plain encoding 0.8).
  - A drop with a whole-store scan (E178's `is_needed`) happened once in 21 copies, at 3,000 reports only. `Repo::commit` itself reads nothing; the whole-store passes are the first write on a fresh handle (ledger and seed index: 3 scans, about 7 reads per stored object) and such drops.
- Reading of the Mac figures (inference, not measured on the Mac): CPU is under 1 ms a copy at 2 reports, so the demo's 16.7 ms of trees is about 3.3 ms for each of the 5 synced tree writes (on Apple systems `sync_all` is F_FULLFSYNC, a flush of the drive's cache), and its 4.7 ms of store is the report's 2.
- Verdict: **measured**. On a small folder a copy's time is its synced writes, not tree work; tree CPU matters only for folders of thousands of files. Next: sync once per commit (E227); skip template search for trees (E228).
- Run: `cargo run --release --example commits [--mem] [--deferred] [--no-object-templates] [--every K] [sizes ...]` (about a minute).

### E227 deferred-sync (objects durable when a ref moves, in the FsStore layout)
- Branch / parent: exp/226-commit-cost / E226
- Question: PackStore (E122) already makes objects durable with one flush when a ref moves, but `ikam live` and its stores on the Mac use FsStore. Keeping FsStore's layout, so existing stores open unchanged, can a burst of writes pay one round of syncs at the commit instead of one sync each?
- Change: `FsStore::open_deferred` (`FsStore::open` unchanged, the default). A write goes to `.pending-<pid>-<n>-<name>` in its object directory, readable at once through the handle; before a ref moves (`set_ref`) and when the handle drops, waiting objects are made durable and renamed into place: on Linux by one `syncfs` after the renames; on Apple systems by a plain fsync of each file and directory, the ref's own F_FULLFSYNC then flushing the drive once; elsewhere `sync_all` of each file and directory. A crash leaves only temporary files, never a partial object under its id, and no ref can name an object not in place. If `syncfs` fails, the objects just placed are removed and the ref does not move. `libc` becomes a dependency on Linux and Apple only. `ikam live --deferred-sync`. Tests (`tests/backends.rs`): the store contract and repository round trip on a deferred store; objects stay under temporary names, unseen by another handle, until a ref moves, and none is left after.
- Result (E226's harness, FsStore against deferred, two runs; bytes and writes identical):
  - A commit after each copy, 2 reports: copy 4.43 / 12.31 to 1.63 / 1.29 ms; 20 copies with their commits 123.6 / 305.8 to 67.6 / 66.9 ms. 300 reports: 138.3 / 268.5 to 125.4 / 91.4 ms. 3,000 reports: 289.5 / 344.6 to 284.8 / 304.8 ms (CPU-bound, see E228).
  - One commit after 20 copies (as `ikam live` commits after a pause): 2 reports 154.7 / 126.8 to 51.4 / 46.7 ms, the commit's one round of syncs 6.2 / 8.3 ms; 3,000 reports 264.9 / 358.2 to 240.9 / 223.8 ms.
  - `ikam live` on a temporary store, three PUTs into `reports/` (with E228's flag): `X-Ikam-Cost` the same new bytes (181, 223, 265), `store_ms` 0.89 to 1.21 against 0.18 to 0.29, `tree_ms` 2.72 to 3.19 against 0.25 to 0.31; after a stop, `ls` and `fsck` clean and no temporary file left.
- Not measured: the Mac. There each of a copy's 9 synced writes is a full flush today; deferred, a PUT does no flush and a commit does one, so the demo's 21 ms of store and trees should fall to about its CPU (under 1 ms). That needs a run there.
- Trade-off: edits are durable when they commit, not when each PUT returns (after a crash, up to `--commit-after` of edits is lost; with FsStore today the objects survive but no ref names them, so the files are lost either way unless someone commits their ids). `syncfs` flushes the whole filesystem, other programs' writes included, so on a busy disk a commit can take longer.
- Verdict: **kept** as an option, off by default. A copy costs about its CPU, and a burst's syncs are paid once at the commit.

### E228 live-tree-slices (`ikam live` planning trees only as slices)
- Branch / parent: exp/226-commit-cost / E227
- Question: E226 found template search is most of a large tree's planning. E144's `Repo::object_templates` (off: trees and view chunks planned only as slices of earlier versions) was never reachable from `ikam live`. What does it change on report copies?
- Change: `ikam live --no-object-templates` sets it. No kernel change.
- Result (E226's harness, MemStore, two runs): bytes a copy identical at every size (trees 464 to 503 B); tree ms 0.52 / 0.51 to 0.30 / 0.27 at 2 reports, 1.44 / 1.02 to 0.60 / 0.77 at 300, 8.26 / 6.75 to 3.03 / 3.01 at 3,000; reads a copy 21 to 17. With E227 on FsStore at 3,000 reports, 20 copies with their commits 284.8 / 304.8 to 202.6 / 243.3 ms.
- Verdict: **kept** as an option, off by default, as E144 left it. What remains at 3,000 files (about 3 ms) is re-encoding and slicing the folder's whole tree; making that proportional to the change needs a tree split into chunks, a format change not tried here.

Gates on the branch: every kernel test suite passes (release); clippy `-D warnings` and fmt clean; the bench gives the same byte and count fields as main 1c5efb9 run on the same machine (only times and peak memory differ), north 0.3035, cost score 0.2456 (main 0.2486, from timings).

### Merge of E226 to E228 into main (Nov 11)
- On Eric's word ("Merge it"), exp/226-commit-cost (c0d333c) fast-forwarded onto main 1c5efb9. `FsStore::open` is unchanged and stays the default; `FsStore::open_deferred`, `ikam live --deferred-sync` and `ikam live --no-object-templates` are off unless asked for.
- Gates, run on c0d333c before the merge (main had not moved): every kernel test suite passes (release); clippy `-D warnings` and fmt clean; the bench gives the same byte and count fields as main 1c5efb9 on the same machine (only times and peak memory differ), north 0.3035.

### E230 mac-commit-cost (E226 to E228 on macOS, APFS)
- Branch / parent: exp/230-mac-results / main 6138eba (numbered after exp/229-db-api's E229)
- Question: E226 to E228 were measured on Linux ext4. E226 inferred that on macOS a copy's time is its synced writes (std's `sync_all` is F_FULLFSYNC there). Is that so, and what do `--deferred-sync` and `--no-object-templates` give on APFS?
- Change: none. E226's harness (`examples/commits.rs`) built from main 6138eba in a temporary worktree on an M3 Pro MacBook (APFS), Nov 11; 20 copies after one warm-up into folders of 2, 30 and 300 reports, then a three-PUT `ikam live` check on a temporary store. The worktree, build and stores were removed afterwards.
- Result (medians a copy; ranges are across 2, 30 and 300 reports; bytes and writes identical in every mode, no drops):
  - Default FsStore, a commit after each copy: store 9.9 to 10.0 ms (2 writes), tree 20.9 to 23.2 ms (5 writes), commit 8.5 to 9.6 ms (2 writes); copy 30.9 to 33.1 ms; 20 copies with their commits 810 to 857 ms. That is about 4.5 ms for each synced write, so E226's inference holds: tree CPU is about 1 ms of the 21.
  - `open_deferred`, a commit after each copy: store 0.9 to 1.1, tree 1.1 to 1.4, commit 4.3 to 5.2 ms; copy 1.9 to 2.5 ms; 20 copies 125 to 155 ms.
  - Deferred with `object_templates` off: tree 0.9 to 1.0 ms, reads a tree 21 to 17; copy 1.7 to 2.1 ms; 20 copies 130 to 140 ms. About 0.2 to 0.4 ms a copy, as small as E228 found at these sizes.
  - A commit after every 20 copies (as `ikam live` does when edits come in a burst): default 616 to 670 ms for the 20; deferred 55 to 67 ms, the one commit taking 16 to 23 ms.
  - A fresh handle's first copy at 300 reports: 115.5 / 122.5 ms default against 45.2 to 46.8 ms deferred.
  - `ikam live --deferred-sync --no-object-templates`, three PUTs into `reports/`: `X-Ikam-Cost` new_bytes 176 / 218 / 260, store_ms 0.22 to 0.36, tree_ms 0.65 to 0.89, index_ms 0.07 to 0.11. After a stop, `ls main:reports` lists r1.md to r3.md (9 B each), `fsck` ok, no `.pending-*` file left.
- Verdict: **measured**. On APFS deferred sync makes a report copy about 15 times faster (31 to 33 ms to about 2 ms) and a burst of 20 about 10 times faster; the trade-off stays E227's (a crash loses edits made since the last commit, which `ikam live` makes after 1.5 s of quiet; a clean stop on SIGTERM commits first). `--no-object-templates` adds little below thousands of files. Both stay off by default in the code.
- Follow-up (Nov 12): `--deferred-sync` turned on for one private `ikam live` deployment on that Mac. After the restart the process carries the flag, the server answers, its log is clean, `fsck` ok and no `.pending-*` file is left while idle.
- Run: `cargo build --release --example commits --bin ikam`, then `commits [--deferred] [--no-object-templates] --every 1|20 2 30 300`.

### E229 db-api (database mode for another program: ranges, an HTTP API, cheaper appends)
- Branch / parent: exp/233-kept-counts (with E231 to E233) / main 9c4c708
- Question: step 1 of the database mode (Nov 9) has a log and a kv view, but only whole scans and point gets, and no way for another program to reach it. What does a first client need, and what does it cost?
- Change:
  - `Repo::graph_range` (graph.rs): nodes with labels in [start, end), in order or reversed, up to a limit, reading only the chunks that can hold them. `db_range`, `db_prefix`, `db_count` (db.rs) on the kv view, reading each event once; `prefix_end`.
  - `ikam db serve` (src/bin/ikam/dbserve.rs): each database NAME is the ref `db/NAME`; `POST append` (binary events, `If-Head` for optimistic writers, every answered append already committed), `GET get`, `POST get`, `GET scan`, `GET count`, `POST span` (count, first and last per prefix in one request), `GET head`, `GET /db/`; reads take `at=COMMIT`. Auth and host checks as `ikam live` (IKAM_AUTH; IKAM_READ_AUTH for GETs only). Listens on 127.0.0.1 only.
  - `Repo::plain_db_events` (off): events stored plain, no plan search. `ikam db serve --plain-events`, `--no-object-templates` (E144's option).
  - Tests: ranges and prefixes equal a filtered scan (6,000 events, random ranges, limits, both directions); `prefix_end`; counts; the server's event round trip, appends, scans, `span`, reads at an older commit, 409 on a moved head.
- Result (`examples/db_append_cost.rs`, MemStore, 10 batches of 2,000 upserts with random row keys): per event over the run 368 us; `plain_db_events` 140 us; `object_templates` off 312 us; both 79 us. Bytes written 5.8 MB in all four (synthetic rows that a plan could not shrink anyway). Most of the default's time is the plan search for each event; `span` answers a list of 20 groups in one request where counts and ends took 60.
- Not measured: real workloads, macOS, rows that share much (where a plan search could pay for itself).
- Trade-off: `--plain-events` can store similar rows larger than a plan would. Every append commits, so a writer that sends one row a request pays a commit a row.
- Verdict: **kept**, both options off by default. Next: aggregate views kept by appends (so a count or a min is not a range read), batching the ids and kv edits across a writer's requests, and a typed value form for rows (JSON arrays today).
- Run: `cargo run --release --example db_append_cost [BATCHES] [PER] [plain] [notemplates]`.

### E231 db-size-order (where a database's bytes go, and what row order costs)
- Branch / parent: exp/233-kept-counts / main 9c4c708
- Question: rows kept in the database mode (E229) take more room than their encoded events, and rows loaded out of key order load far slower. Where do the bytes go per row, and how much does order matter?
- Change: none in the kernel. `examples/db_size_breakdown.rs` writes grouped numeric rows (a JSON array of nine fields under `t/rows/GROUP/SEQ`, idempotency keys as a bulk load makes them) into pack stores, batched (2,000 an append, a commit each) or in one append, in key order or shuffled, and prints the parts of an event and the bytes on disk.
- Result (300 groups x 1,200 rows, 360,000 rows, `--plain-events` and `--no-object-templates`):
  - Parts: values 47.8 MB, keys 6.8 MB, idempotency keys 16.6 MB; encoded events 80.2 MB. On disk 136.6 MB batched (379 bytes a row, 1.70x the events) and 135.5 MB in one append, so commits are not the cost. With object templates on: 136.7 MB, and the load takes 26 s against 17 s.
  - Per row the log repeats the key inside the event, keeps an idempotency key of about 46 bytes, and the three views add about 150 bytes; events of 200 to 250 bytes are too small for their encoding to find anything to share.
  - Order (100 groups x 1,200 rows, batched): shuffled rows take 186.7 MB and 90.8 s against 45.5 MB and 4.3 s in key order (4.1x the bytes, 21x the time), because every append rewrites chunks all over the views and the old versions are kept. In one append the order does not matter (45.1 MB both ways). A bulk load that writes each table in key order avoids the cost.
- Trade-off: none taken; a measurement.
- Verdict: **measured**. Next: columnar blocks for large tables, shorter or dropped idempotency keys for bulk loads, kept counts so a list reads no ranges (E233).
- Run: `cargo run --release --example db_size_breakdown [GROUPS] [PER_GROUP] [plain] [notemplates] [shuffle] [edit-index]`.

### E232 fast-writes (appends that do not wait for reads, and a cheap first write after a restart)
- Branch / parent: exp/233-kept-counts / main 9c4c708
- Question: small appends to `ikam db serve` took 50 ms and more in an end-to-end test while list views were being read. A write path in the style of Rama's depots answers an append as soon as the log has it, whatever readers are doing. What does an append cost in ikam itself, and what makes it slow?
- Change:
  - `examples/db_write_path.rs` (an append's steps on a pack store holding 200,000 rows, and the first append after reopening) and `scripts/db_contention.py` (readers loop over a span of 200 prefixes and one prefix's scan while a writer appends a row every 50 ms; standard library only).
  - `SharedStore` (store.rs): a store several `Repo`s share, each call locking for one object or ref. `Repo::sibling`: another handle on it that shares the ledger, so it reads chunks the writer derived after it started. The ledger is now `Arc<RwLock<Option<Ledger>>>` behind `ledger()`, shared by siblings and unchanged for a single handle.
  - `ikam db serve --concurrent-reads` (off): reads run on sibling handles from a pool; only appends take the writer's lock. Every read still works from the roots of the head it saw when it started.
  - `ikam db serve --edit-index` (off): plans each write against the chunks the edit read (E133's empty scope with replace hints) instead of an index of every stored object.
  - Test: a sibling reads what the writer derived after the sibling built its ledger (fails with an unshared handle: the newer chunks are not found).
- Result (pack store, 200,000 rows over 200 groups, 68.3 MB, plain events, no object templates):
  - An append alone costs about 4 ms in ikam (the event, log, ids and kv edits 3.3 ms, the commit with its sync 0.9 ms) and about 0.5 ms more over HTTP. The 50 ms were waiting: each request took the server's one lock, so an append queued behind every read in flight.
  - With 3 readers and a writer, default: appends p50 305 ms (p95 393), 3.5 a second against the 20 asked for; span p50 355 ms, scan 212 ms, 10.5 reads a second.
  - With `--concurrent-reads`: appends p50 7.5 ms (p95 11.1), all 20 a second; span 176 ms and scan 22 ms, about the same as with no writer (168 and 24); 31.3 reads a second.
  - The first append after a restart took 4.5 to 5.1 s: the first plan builds an index of every stored object. With `--edit-index` it takes 4 ms in the example and 0.48 s over HTTP (most of it the ledger, built once), and the server's peak resident memory is 30 MB against 172 MB; steady appends and bytes on disk are unchanged (68.3 MB both ways; E231's 360,000 rows 136.6 MB both ways, and 136.7 MB both ways with object templates on).
- Not measured: several writers at once (group commit is the next step), real workloads, macOS.
- Trade-off: `--concurrent-reads` takes a lock per object read and keeps a handle per concurrent reader. `--edit-index` cannot slice a new chunk against stored contents this edit did not read; on these rows that saved nothing.
- Verdict: **kept**, both off by default. Next: group commit for several writers, and views kept by appends so lists read counts instead of ranges (E233).
- Run: `cargo run --release --example db_write_path -- DIR [ROWS] [APPENDS] [PER] [edit-index]`; `python3 scripts/db_contention.py URL [--readers 3] [--appends 200]` against `ikam db serve --packs --plain-events --no-object-templates` on a copy of a store written with `APPENDS` 0.

### E233 kept-counts (a view of how many keys fall under each prefix, kept by appends)
- Branch / parent: exp/233-kept-counts / main 9c4c708
- Question: a list view asks for the number of rows under each of a few hundred prefixes (one per group). `db_count` reads every key in a prefix's range, so the list costs as much as reading the table's keys, and grows with it. Rama keeps such sizes in its indexed views; can the database mode keep a count per prefix, edited by each append, and read it in one pass?
- Change:
  - `Db::counts: Option<Counts>`: a fourth view, from each key prefix of 1 to `depth` segments (`t/`, `t/rows/`, `t/rows/G0007/` for depth 3) to the number of keys starting with it, as decimal content; a prefix with no keys has no node. A commit stores it as `db/views/counts` with `counts.at` and `counts.depth`, only when kept, so a database without it commits exactly as before.
  - `db_append` edits it from the keys whose presence an append changes (read from the chunks of `kv` those keys fall in); `db_counts_rebuild` builds it from `kv`'s labels, the reference the kept view must equal; `db_keep_counts(db, depth)` turns it on for an existing database (0 turns it off).
  - `db_count_prefixes` (and `db_count_prefix`): kept prefixes in one pass over the view's chunks, any other prefix by its range as before.
  - `ikam db serve --keep-counts N` (off): builds the view on a database's next append when it has none; `count?prefix=` and `span` use it.
  - `examples/db_kept_counts.rs`; tests: kept counts equal counts from scratch after new keys, overwrites, removes, subtree removes and retries over 30 rounds, every prefix's kept count equals its range count, a commit keeps the view, turning it on later builds the same view, and the server's routes answer the same with and without it.
- Result (pack store, 200,000 rows over 200 groups, plain events, no object templates, depth 3):
  - Counts of the 200 groups and the table: 243 ms by range, 0.42 ms kept. Over HTTP with `--concurrent-reads --edit-index`, `span` of the 200 groups (count, first and last pair each) 170 to 51 ms p50 alone and 167 to 51 ms with a writer, and reads a second 32.5 to 83.9; the rest of a span is its 400 first and last reads.
  - Building the view after a load: 0.18 s; keeping it during the load: 8.3 s against 6.8 s. Bytes on disk after either: unchanged at 57.2 MB to the first decimal.
  - One-row appends with commit: p50 3.2 to 5.1 ms; over HTTP with 3 readers, 8.0 to 9.5 ms. 200 appends grew the store 1.5 MB against 1.3 MB without the view (the view's chunk is rewritten each time).
- Trade-off: about 2 ms and 1 KB more per small append, for counts that no longer grow with the table. Only prefixes ending in `/` of up to `depth` segments are kept; any other range counts as before.
- Verdict: **kept**, off by default. Next: first and last pair per prefix kept the same way (the rest of `span`), and counted interior nodes for any range.
- Run: `cargo run --release --example db_kept_counts -- DIR [ROWS] [APPENDS] [off|after|load]`; `python3 scripts/db_contention.py URL` against `ikam db serve --keep-counts 3` on a store written with `after`.

Gates on the branch (E229 to E233): every kernel test suite passes (release); clippy `-D warnings` and fmt clean; the bench gives the same byte and count fields as the code it builds on, run alongside it on the same machine (only times and peak memory differ), north 0.3109; the privacy check passes.
