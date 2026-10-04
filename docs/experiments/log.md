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
