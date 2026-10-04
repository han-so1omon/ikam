# Research note: compact storage of versioned trees / manifests (2026-10-04)

**[read]** means I read the text myself. **[inf]** means my own inference. **[measured]** is a one-off measurement I ran with stock git, not a published number. Sources are cached in the session scratchpad and are not committed.

## Question

Our tree object is a flat sorted list of entries: `name_len:u32 name kind id[32]`. In the 7-snapshot repo benchmark, the 7 trees take **203 KB after zstd+dict (~15% of the store)**. They are nearly identical from one snapshot to the next, but each is stored whole. One 343-file tree costs **14 KB**, and the 32 B BLAKE3 ids make up most of that. zstd cannot shrink the ids: they are random bytes, so they only shrink if they are *referenced* rather than *repeated* [inf]. How do established systems store versioned directory listings compactly, and what numbers do they report?

## Papers read

**1. Mackall, "Towards a Better SCM: Revlog and Mercurial", OLS 2006, vol. 2, pp. 91–98.** https://www.kernel.org/doc/ols/2006/ols2006v2-pages-91-98.pdf. **Full text [read].**
- Every history is a *revlog*: a fixed 64-byte index record per revision plus an append-only data file of hunks. Each hunk is either a full text or a delta against the preceding hunk(s), "like MPEG … occasional full frames."
- A delta chain is bounded: its total length must stay within "a small multiple" of the revision's uncompressed size. Retrieval is therefore O(1) seeks with bounded CPU.
- The *manifest* is a sorted list of `file:revision-hash` pairs, one per project revision, and it is stored in a revlog like any file. Consecutive manifests are stored as deltas.
- Deltas use `bdiff` (difflib-style longest-match). It produced "slightly smaller output than either the Myers or xdelta algorithms" and was as fast or faster.
- Typical per-file compression is "10:1 to 20:1" (weave/delta plus gzip). The paper gives **no manifest-specific size numbers**.
- Maps to us: store tree *n* as a delta against tree *n−1*, with a periodic full tree and a bounded chain. Entries whose ids are unchanged become copy ops.

**2. Mercurial internal docs (mercurial 6.8 sdist, `helptext/internals/revlogs.txt` and `requirements.txt`; `manifest.py`). Full text of the relevant sections [read].**
- Delta chains are "limited to ~2x the length of the revision's data". Read distance is bounded as well.
- *generaldelta* (the default since 3.7, 2016) deltas against a parent rather than the previous revision.
- A manifest line is `path \0 hex(node) flags \n`, so ids are stored as **40-char hex text**. Line-based deltas still make it cheap, because unchanged lines are never re-stored [inf].
- *treemanifest* (one manifest per directory) is still flagged experimental.

**3. Mercurial wiki, TreeManifestPlan (archived, last edited 2016).** https://wiki.mercurial-scm.org/TreeManifestPlan. **Full page [read].**
- Each directory gets its own manifest revlog and its own nodeid. Untouched directories are not re-hashed, delta chains get shorter, and diffs can skip identical subtrees. Entries store only the basename.
- Cost: "+7% number of revlogs for the Mozilla repo, +20%/80k for a Facebook repo." Changing a deep file writes one revlog per level.
- **No storage-size numbers** are given.

**4. Git pack format and packing heuristics.** Full text [read] of `Documentation/gitformat-pack.adoc`, `technical/pack-heuristics.adoc`, `git-gc.adoc`, `config/gc.adoc`, `config/pack.adoc` and `git-pack-objects.adoc` (git master, raw.githubusercontent.com).
- **Format:** trees are deltified like blobs, as `OFS_DELTA` (base given as a negative pack offset) or `REF_DELTA` (base given as a 20 B id).
  - A delta is a sequence of *copy* ops (1 byte header, then up to 4 offset bytes and 3 size bytes; the smallest form is the single byte `0x80`) and *insert* ops (≤127 literal bytes).
  - Each object is zlib-compressed on its own to keep random access. Linus: "the actual compression factor is less than it could be in theory."
- **Heuristic:** candidates are sorted by type, then a path "name hash", then size (largest first). A sliding window tries to delta each object against the last *window* objects.
  - Defaults: `pack.window` 10, `pack.depth` 50; `gc --aggressive` uses window 250 and depth 50. Linus: "removing data is cheaper than adding."
- **Path-aware delta selection** (Stolee, commit messages `fc62e03` and `5f71150` in git.git [read]):
  - `--path-walk` (deltas among objects at the *same full path* first): fluentui 439.4 M (v1) / 161.7 M (v2) / 142.5 M; git.git 248.8 → 213.2 M; Linux 2.5 → 2.2 G.
  - These totals mix blobs and trees and are not split out per type.
- **Nested trees:** git trees are per-directory, so an unchanged subdirectory is shared by id across commits for free. That is the same idea as treemanifest.
- **[measured]** ripgrep clone after `repack -adf --window=250 --depth=50`:
  - 6,351 trees, 3.14 MB raw → **0.46 MB on disk (6.8×)**.
  - 5,371 / 6,351 trees (85%) are deltified. Trees are **13.6% of the pack**, close to our 15%.
  - This is one repo; I did not find a published per-type breakdown.

**5. Meister, Brinkmann, Süß, "File Recipe Compression in Data Deduplication Systems", FAST 2013.** https://www.usenix.org/system/files/conference/fast13/fast13-final54.pdf. **Full text [read].**
- Problem: a recipe is a list of 20 B fingerprints, one per 8 KB chunk. With daily full backups kept for a year, recipes reach about 17.1 TB against 43 TB of deduplicated data, which is "about 40%" of it.
- Datasets:

  | Dataset | Backups | Logical size | Dedup ratio |
  |---|---|---|---|
  | HOME1 | 15 weekly | 6.6 TB | 1:20 |
  | HOME2 | 5 weekly | 4.8 TB | 1:6 |
  | ENG | 21 weekly | 319 GB | 1:40 |

- **ZC, zero-chunk suppression:** a 1 B code or bitmask replaces the zero chunk. Recipe shrink alone: ENG 12.6% in Table 3 (11.8% in the text; the paper is inconsistent), HOME1 5.4%, HOME2 2.6%. The zero chunk is 14.9% / 0.6% / 0.3% of references respectively.
- **PB, chunk-index page-based code words:** code = page number (prefix) ‖ the shortest suffix that is unique within that page.
  - At 8 TB with 2^24 pages of 4 KB (about 64 chunks per page, at most 97 w.h.p.) this is a 31-bit, i.e. **4 B**, code. At 256 TB it is 5 B.
  - The code is an alternate key into the chunk index, so lookup needs no extra index access.
  - Saving: **80%** (75% at 5 B), independent of the data.
- **SD, statistical dictionary:** order-0 entropy, average 17.3 bits/chunk on ENG. Chunks whose entropy is below 85% of log2(n) get 24-bit codes.
  - Only 0.07–0.36% of chunks get a code. Saving: 14.2% / 9.0% / 3.6%.
  - Needs an in-memory reverse index of about 128 MB per 8 TB.
- **SP, statistical prediction:** order-1 entropy is only **0.13–0.21 bits/chunk**, because runs of known chunks are long: averages of 449.8 / 262.9 / 901.5.
  - Each chunk-index entry keeps a Misra–Gries summary (k=2) of its likely successor. If the next fingerprint equals the prediction, a **1 B code** is stored.
  - A full fingerprint is stored every **b=32** entries as a random-access anchor.
  - Saving alone: ENG 82.2%, HOME1 77.3%, HOME2 69.2%. k=4 adds about 0.3 points. A full order-1 model reaches only 82.6 / 77.6 / 69.5%.
  - Cost: about 68 B per index entry (about 20 B combined with PB).
- **Combined:** all four give **93.3% (ENG), 92.3% (HOME1), 90.9% (HOME2)**. PB+SP alone is "within 0.3%" of that. ZC and SD add almost nothing once PB+SP are in place.
- Maps to us: our tree is a recipe whose id list is 32 B per entry. PB corresponds to short store-local ids. SP corresponds to "same entry as in the predecessor tree". This is the same redundancy that revlog and git deltas exploit, encoded per entry rather than per byte [inf].

## Hypotheses

**H1. Delta-chained trees (sources: Mackall/revlog §3–4; git pack-format OFS_DELTA; [measured] 6.8× on ripgrep trees).**
- Change: in `repack`, encode each tree as a byte delta (copy/insert) against the tree of the parent snapshot.
  - Use a full base every N trees, or whenever the chain exceeds 2× the tree's size (the Mercurial rule).
  - Apply zstd+dict to the delta stream. The tree's id stays the hash of its full canonical bytes.
- Expected effect: about 1 full tree plus 6 small deltas, instead of 7 full trees. If ≤5% of entries change per snapshot, that is roughly 203 KB → 35–50 KB [inf].
- Gates: byte-exact reconstruction, and fsck must verify every reconstructed tree's hash.
- **Status:** confirmed by E005, in a different form: trees stored as content and derived by slices against earlier trees (no chain-length cap needed so far), repo-history trees 203,414 B -> ~32 KB (estimated from totals), score 0.6643 -> 0.6491.

**H2. Entry-level prediction from the predecessor tree (source: Meister SP, 69–82% alone).**
- Change: store a tree as a reference to its base tree plus a sorted edit list (delete name / upsert entry), so a predicted entry costs ~0 bytes.
  - This is the structural version of H1. Meister's b=32 anchor corresponds to bounding the chain.
- Expected effect: similar to H1, and possibly smaller, because there are no copy-op headers and no insert of a 32 B id when only the name shifts [inf].
- Run it as a separate branch from H1 (one hypothesis per branch). Compare it against H1.
- **Status:** `open` (not yet run).

**H3. Store-local short ids inside stored trees (source: Meister PB, 80% data-independent; git OFS_DELTA uses offsets, not ids).**
- Change: in the stored (not hashed) form, replace each 32 B entry id with a varint ordinal into the pack's object table. The canonical form, and the hash, keep the full ids.
- Expected effect: id bytes go from 32 to about 2–3 per entry. The 343-entry tree goes from about 14 KB to about 4–5 KB before zstd. It helps every tree, including the first.
- It stacks with H1/H2: under a delta, only the changed entries' ids remain.
- **Status:** `open` (not yet run).

**H4. Nested per-directory trees (sources: git trees; Mercurial treemanifest, which reports +7% revlog count on Mozilla).**
- Change: stop flattening paths into names, so unchanged subdirectories dedupe by id across snapshots.
- Expected effect: only the trees along changed paths are rewritten each snapshot. The cost is more objects and per-object framing.
- Flattening is done by the benchmark (`examples/bench.rs` `commit` replaces `/` with `__`), not by the kernel, which supports nested trees. Changing it changes the measurement, not the kernel, so H4 would need a protocol decision. (`unshare_trees` is unrelated: it returns unshared trees to object form; see E005.)
- **Status:** `open` (not yet run).

## Unknowns

- Not found: any published per-object-type size breakdown for git (the share of pack bytes that are trees) in Scalar, VFS for Git or sparse-index docs. The sparse-index doc has no size numbers.
  - lore.kernel.org and repo.mercurial-scm.org returned bot-check pages. I used the git.git commit messages and the Mercurial PyPI sdist instead.
- The Mercurial treemanifest page reports timings only, not storage size. I found no manifest-specific compression numbers for Mercurial.
- Meister's numbers come from backup traces with 8 KB chunks and very long duplicate runs. Our trees have about 343 entries and 7 versions, so the 90%+ figures are an upper-bound analogue, not a prediction [inf].
- The H1 and H3 effect estimates assume the fraction of changed entries per snapshot. That fraction has not been measured on our corpus [inf].
- The ripgrep figure is a single measurement of nested git trees using SHA-1 ids (20 B). It is not a property of our flat 32 B-id trees.
