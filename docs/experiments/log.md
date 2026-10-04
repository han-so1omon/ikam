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
