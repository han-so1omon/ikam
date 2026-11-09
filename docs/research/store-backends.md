# Research note: object-store backends and access paths (E122 to E127, 2026-11-09)

**[read]** means I read the text myself. **[inf]** means my own inference. Read in-session; not cached. OpenResearch (`orx`) could not be installed in this environment (its installer was refused by the network proxy, 403), so sources were found with web search and read at their own pages; the experiments follow the README's git-branch loop instead.

## Question

Which `Store` backend and on-disk layout fits ikam? ikam stores write-once BLAKE3 objects (mostly 4–32 KB prolly chunks) plus refs moved by CAS. It runs on APFS now and maybe on R2 later. What do git, dbt, journaling, log-structured, copy-on-write (COW), versioning, hierarchical and virtual designs teach, and which earn a slot beside E122's files, packs, SQLite WAL, LMDB, redb, fjall and RocksDB?

Repo fact (`store.rs` **[read]**): `write_atomic` writes a tmp file, calls `sync_all`, then `rename`. It never fsyncs the parent directory.

## Sources read

**1. git: pack-format v2.33.2, multi-pack-index, commit-graph format, git-gc (git-scm.com, kernel.org) [read].**
- The `.idx` v2 layout is a 256-entry fanout ("fanout[255] is the total object count"), then sorted names, a CRC32 per object, 4-byte offsets, and an 8-byte offset table only for packs over 2 GiB.
- The multi-pack index (MIDX) gives "O(log N) lookup time for any number of packfiles". Incremental chains exist, but compacting chain layers is "not yet implemented".
- gc defaults: gc runs at 6700 loose objects or 50 packs. Unreachable objects get a "2.weeks.ago" grace period, and `--prune=now` "increases the risk of corruption".
- **Borrow:**
  - The fanout + sorted ids index (= `ids_with_prefix`).
  - A CRC per entry.
  - gc triggered by object or pack counts, with a grace period before delete.
- **Status:** packs are already in the bake-off. Add a MIDX-style chain variant.

**2. git reftable spec [read].**
- Android: packed-refs 62.2 MB vs reftable 36.1 MB; cold lookup 409,660 µs vs 33.9 µs.
- Commits rename `tables.list.lock` to `tables.list`, the single commit point.
- **Borrow:** one renamed manifest when several refs move together.
- **Reject:** the format itself, since ikam has few refs [inf].
- **Status:** note only.

**3. dbt: incremental models, selection methods, defer (docs.getdbt.com) [read].**
- `is_incremental()` is true only if the table exists, the model is incremental and `--full-refresh` is absent.
- The docs admit that new rows "may diverge from the historical transformations … you should rebuild."
- `state:modified` diffs against a saved manifest. Seeds of 1 MiB or more are flagged only on a path change.
- `--defer` resolves unbuilt upstreams from another environment.
- **Maps to us [inf]:**
  - A view position is dbt's high-water mark, and a full refresh is a re-derive from position 0.
  - Content ids make "modified" exact: hash the function id with the input ids.
  - Defer means taking an already-rendered output from another ref.
- **Reject:** dbt's tolerated divergence. The byte-perfect rule means incremental must equal full.
- **Status:** note only (S7).

**4. ext4 admin guide (kernel.org) [read]; Pillai et al., OSDI 2014 (wisc PDF) [read].**
- ext4 `data=ordered` (default) forces data out "prior to its metadata being committed"; commit interval 5 s.
- The ALICE tool found 60 static vulnerabilities across 11 applications: Git 9, SQLite rollback 1, SQLite WAL 0, LMDB 1, LevelDB 1.10 10.
- Three findings that matter:
  - "An fsync on a file does not guarantee that its directory entry is persisted."
  - Renames look atomic on journaling and COW file systems.
  - Multi-block appends are not atomic.
- **Borrow:** fsync the directory after renames, once per batch. In logs, store a CRC per record and truncate at the first bad one.
- **Reject:** relying on ext4 heuristics (the Mac runs APFS).

**5. Apple `fsync(2)` man page (developer.apple.com) [read]; Rust PR #60121 plus the "Auto merge of #60121" diff (github, fuchsia mirror) [read].**
- Apple: after fsync the drive "may not physically write the data … and it may be written in an out-of-order sequence … This is not a theoretical edge case." F_FULLFSYNC "asks the drive to flush all buffered data".
- Rust: after that PR, `File::sync_all` on macOS calls `fcntl(F_FULLFSYNC)`. The PR says `sync_data` already did.
- **Implication [inf]:** FsStore pays a full drive flush for every 4–32 KB object on the Mac. That is probably its dominant write cost.
- Second-hand (mjtsai.com) [read]: Hector Martin, "Apple SSDs are 1000x slower when flushing"; Scott Perry suggests `F_BARRIERFSYNC` for ordering.
- **Borrow:** flush once per batch, before the ref CAS (S1).

**6. Rosenblum & Ousterhout, LFS, TOCS 1992 (berkeley PDF) [read].**
- Write cost is 2/(1−u), where u is the utilization of cleaned segments.
- The cost-benefit cleaner scores segments by (1+u)/((1−u)·age) and does up to 50% better than greedy. Production write cost was 1.2–1.6.
- **Borrow:** clean packs by live fraction × age. **Status:** pack GC policy; note only.

**7. O'Neil et al., LSM-tree, 1996 (umb.edu PDF), ~85% [read].**
- Batches inserts in memory and rolling-merges to disk, claiming "an order of magnitude lower cost" than a B-tree.
- **Maps to us [inf]:** ikam's keys are random and its values large, so value rewriting is the cost to watch (8).

**8. Lu et al., WiscKey, FAST 2016 (usenix PDF) [read].**
- LevelDB write amplification measured 3.1 at 1 GB and 14 at 100 GB. Read amplification measured 8.2 and 327.
- WiscKey loads 2.5–111× and looks up 1.6–14× faster; ~12× slower on 64 B range scans.
- **Borrow:** key/value separation. ikam never overwrites values, so only deletes leave garbage [inf].
- **Status:** RocksDB and fjall are already in the bake-off, but separation must be switched on (S4).

**9. Bitcask: HighScalability summary [read]. The paper itself was not reachable.**
- Writes append to a log. An in-RAM keydir maps each key to a file and offset, so a read is "one seek". Merges plus hint files keep restarts fast. "All keys must fit in RAM."
- **Maps to us [inf]:** 32-byte ids at 10M objects ≈ hundreds of MB of RAM; a sorted keydir serves prefix lookups.
- **Status:** **worth adding** as the simplest pack variant (S3).

**10. RocksDB integrated BlobDB blog (2021) [read].**
- Write amplification drops from 1.6 to 1.0–1.02 on load, and from 6.1–6.8 to 1.4–1.7 on overwrite. Load is 2.3–4.7× faster.
- **Status:** already in the bake-off. Run it with blobs on and off.

**11. ZFS on-disk spec (freebsd PDF), Btrfs design (readthedocs), APFS Guide plus "About Apple File System" (developer.apple.com) [read].**
- ZFS: 128-byte block pointers carry a 256-bit checksum and the birth transaction group. The live uberblock is the one with the "highest transaction group number and valid SHA-256 checksum", and it is never overwritten.
- APFS: "copy-on-write metadata … without the write-twice overhead of journaling". Clones "occupy no additional space".
- **Borrow [inf]:**
  - Treat the ref as an uberblock: checksummed, with the previous value kept.
  - `clonefile` for O(1) store snapshots (bake-off resets).
- **Status:** note only.

**12. Dolt: block-store docs, the "Journaling Chunk Store" post (2023), the "Storage Review" post (2024) (dolthub.com) [read].**
- A table file is chunk records, then an index (sorted 8-byte prefixes, lengths, 12-byte suffixes), then a footer. Ids are SHA-512 truncated to 20 bytes, and chunks are Snappy-compressed.
- The chunk journal appends to one file instead of a file per transaction; sysbench writes went from 3.7× to 1.3× MySQL (baseline also changed).
- **Borrow:** the prefix-array index, and a journal for hot writes beside table files. It is the closest prior art to ikam.
- **Status:** the design target for packs (S2).

**13. lakeFS versioning internals (v1.64 docs) [read].**
- A commit is a metarange over content-addressed SSTable ranges kept in the object store. At 5–20% daily change, commits "reuse >= 99%" of blocks.
- Branches live in a KV store "for consistency guarantees". **Borrow:** immutable objects in the bucket, refs where CAS is strong. **Status:** note only.

**14. Iceberg spec, goals only [read].** Goals only ("never partially visible"). Note only.

**15. SQLite `wal.html` and "35% Faster Than the Filesystem" (sqlite.org) [read].**
- WAL mode:
  - It needs "many fewer fsync() operations".
  - It allows one writer and has no network-filesystem support.
  - Under `synchronous=NORMAL`, transactions "might rollback following a power failure".
- The blob test, 100k blobs of 8–12 KB:
  - Reads were about 35% faster than files, and the database used about 20% less space.
  - With fsync per file, file writes were "about 10 times or more slower" than SQLite.
  - Sharding to 100 files per directory or fewer made "very little performance difference".
- **Status:** already in the bake-off. Test both `synchronous` levels.

**16. LMDB `mdb(3)` man page [read]; Wikipedia [read, secondary].**
- Map size caps the database; keys ≤ 511 B; long readers block page reuse. COW B+tree, one writer.
- **Status:** already in the bake-off.

**17. redb README v4.0.0 [read].** On a Ryzen 9950X3D with a 9100 PRO SSD:

  | | redb | lmdb | rocksdb | fjall | sqlite |
  |---|---|---|---|---|---|
  | individual writes (ms) | **920** | 1598 | 2432 | 3488 | 7040 |
  | batch writes (ms) | 1595 | 942 | 451 | **353** | 2625 |
  | random reads, 32 threads (ms) | 410 | **125** | 1100 | 576 | 26536 |
  | compacted size | 1.69 GiB | 1.26 GiB | **454.71 MiB** | 1000.95 MiB | 556.85 MiB |

- Value sizes unstated; not our workload [inf]. **Status:** in the bake-off.

**18. fjall README v3.0.4 [read].** LSM, optional KV separation; writes reach "OS buffers but not disk" until `persist(SyncAll)`. **Status:** in the bake-off, with separation on.

**19. "Billion-files File Systems", arXiv 2408.01805 [read].**
- ext4, 1–10 KB files, 100k per folder: 24 µs per create, 30% space overhead; default ~240M inodes forced `mkfs -N`.
- **Borrow:** keep 2-hex sharding; deeper is unjustified (also 15).

**20. Cloudflare R2 pricing, limits and consistency; S3 performance guide [read].**
- R2 prices: $0.015 per GB-month. Class A (Put, List, Copy) is $4.50 per million, Class B (Get, Head) $0.36 per million. Deletes are free.
- R2 behavior:
  - Writes to one key are capped at "1 per second".
  - Reads see a completed write immediately.
  - `If-Match` and `If-None-Match` on PutObject are evaluated atomically.
- S3: at least 3,500 writes/s per prefix, unlimited prefixes.
- **Implications [inf]:**
  - 10 GB of ~8 KB chunks is about 1.3M PUTs, roughly $5.90, against $0.15 per month of storage. As 64 MB packs it is about 160 PUTs.
  - `ids()` must read a stored index, never LIST.
  - `set_ref` maps to a conditional PUT, at most 1 move/s per ref.
- **Status:** worth adding a counting R2 cost model (S6).

**21. Virtual layers (all [read]):** GitHub's "Story of Scalar", the overlayfs kernel docs, Apple's File Provider (replicated extension) docs, macFUSE 5.0.0 notes, and PostgreSQL's `REFRESH MATERIALIZED VIEW`.
- VFS for Git loaded files "only when a filesystem read occurred". Its Mac port died because "Apple deprecated the kernel features" it needed. Scalar replaced it with cone-mode sparse checkout (`git status` in "three or four seconds").
- overlayfs: upper wins, writes copy up, whiteouts hide. File Provider: "the system takes responsibility for managing and storing the local copies". macFUSE's FSKit backend needs macOS 15.4+. Postgres REFRESH "completely replaces the contents".
- **Maps to us [inf]:**
  - Keep WebDAV in user space and reject kext FUSE. File Provider is the native hydrate-on-read path if Finder latency hurts.
  - A branch is already an overlay: a ref plus a delta.
  - ikam's views are incremental materialized views. Render-on-read trades store bytes for latency.
- **Status:** note only (S8).

## Comparison table

| Layout | Durability cost per write | Prefix lookup | Reclaim | Crash evidence | R2 fit | E122 |
|---|---|---|---|---|---|---|
| FsStore loose, 2-hex | F_FULLFSYNC per object (5) | readdir of shard | unlink | dir entry unsynced (4) | 1 PUT per object (20) | in |
| Packs + fanout index (git/Dolt) | 1 flush per batch [inf] | fanout + bsearch (1, 12) | cost-benefit repack (6) | prefix-recoverable appends (4) | packs + index objects | in |
| Bitcask log + keydir | append; index in RAM | sorted keydir [inf] | merge (9) | not read | as packs | **add** |
| SQLite WAL | few fsyncs (15) | B-tree range | VACUUM [inf] | 0 vulns (4) | no | in |
| LMDB / redb | COW B-tree, 1 writer (16, 17) | range | free-page reuse | LMDB 1 vuln (4) | no | in |
| fjall / RocksDB | LSM; blob write amp ~1.0–1.7 (10) | prefix scan | compaction / blob GC | LevelDB 1.10: 10 vulns (4) | no | in, separation on and off |

## Hypotheses

**S1. Batch durability (sources 4, 5).**
- Change: replace per-object `sync_all` with `libc::fsync` or F_BARRIERFSYNC. Before `set_ref`, run one F_FULLFSYNC and fsync the directory.
- Expected effect: much faster APFS ingest (no primary number).
- Measure: objects/s over a 1M-chunk ingest. After a crash, every object reachable from a ref must be present and hash-valid.
- **Status:** partly confirmed by E122 (Linux, so not the Mac's cost): a single-event commit makes 66 flush calls on FsStore (one per object written) against 2 on packs and 1.3 on SQLite; on macOS each is an F_FULLFSYNC. Batch durability without packs (FsStore with one flush per ref) was not built. Needs a run on the Mac.

**S2. Native packs with a Dolt/git index (sources 1, 6, 12).**
- Change: append-only packs with a CRC per entry. Index each with a fanout plus sorted ids and offsets, chained like an incremental MIDX. Delete marks entries dead; a cost-benefit cleaner reclaims them.
- Expected effect: no per-file overhead (~30% on ext4), fast prefix lookup.
- Measure: ingest, read latency, on-disk/logical bytes and repack time at 1M and 10M objects.
- **Status:** confirmed by E122. Packs write the 229 MB corpus at 769 MB/s (files 273), read it cold at 3.3 GB/s (files 0.83), change 2 files per commit (files 25), hold 1M small objects in 372 MB (files allocate 4.1 GB), and lost nothing in 50 kills. Opening 1M objects took 44 s until the open pack's index was checkpointed (196 ms; 264 ms at 10M). The cleaner and a multi-pack index are not built.

**S3. Bitcask variant (source 9).** Change: S2 without an on-disk index; a sorted keydir is rebuilt from hint files. Measure: open time and RSS at 10M ids. **Status:** not applicable after E122: the open pack's in-memory index already is a keydir, and its checkpoint is the hint file; sealed packs need no RAM index.

**S4. Key/value separation on the LSM engines (sources 8, 10, 18).**
- Change: RocksDB `min_blob_size` around 4 KB; fjall with separation on.
- Expected effect: write amplification near 1.0, against 1.6 or more without it.
- Measure: device bytes written ÷ logical bytes.
- **Status:** measured in E122, small effect at our sizes: fjall with separation read the corpus cold at 1.1 GB/s against 0.46 without; RocksDB blobs changed nothing measurable (0.63 against 0.61 GB/s). Device bytes written were not measured (no block-layer counters here).

**S5. Checksummed refs (sources 2, 11).** Change: a ref holds id + checksum + previous value; multi-ref moves use one renamed manifest. Measure: torn-ref fault injection. **Status:** open (no torn ref seen in 500 kills). The kill test found a different ref fault instead: FsStore's lock file, held by existence, wedged a ref forever after a kill mid-update (22 of 25 children refused); E122 locks with an OS lock.

**S6. R2 cost model (source 20).**
- Change: a Store wrapper that counts Put, Get, Head and List per kernel operation, for loose files and for packs.
- **Status:** measured as store reads in E124 to E127 (a counting store): a small file costs 2 GETs (its entry and its group), a docx 31 and a pptx 59, a key lookup in a 100k-key view 10 (2.4 with a handle cache). R2 prices then follow; see the design doc.

**S7. State-selected derivations (source 3).**
- Change: skip a derivation when its key (function id, input ids) already has an output. Defer missing outputs to another ref.
- Measure: rebuild time after changing 1% of inputs. The incremental view id must equal the full-rebuild id.
- **Status:** open (E119 and E120 already hold the law for the db views: in-place edits equal a full rebuild).

**S8. Materialized vs render-on-read views over WebDAV (source 21).** Measure: Finder listing/open p50/p99 against store bytes. **Status:** open; needs the Mac.

**S9. Verified range reads (BLAKE3's own tree; Bao outboard) [inf].** BLAKE3 hashes 1 KiB chunks into a binary tree, so a range can be verified against the content id with the sibling hashes along its path. With 16 KiB chunk groups the hashes are 64 bytes per group (0.4% of the content). Today a range of a content stored whole is verified by rebuilding all of it: a 4 KiB range of a 1 MB png fetched 2.2 MB, 1.1 MB after E124's fix. With an outboard tree and ranged store reads (pread, an R2 Range GET), it would fetch about 32 KB and 0.4 KB of hashes. **Status:** open; a format addition, so it waits on Eric.

**S10. Mixed objects stay inside one permission domain [inf].** Groups, the dictionary and delta bases are stored objects whose bytes hold several files' content. On the cases corpus 29 such objects serve reads of two or more of 24 case folders, one of them all 24 (E126), so per-tenant storage keys, buckets or presigned URLs cannot isolate tenants unless grouping and deltas stay inside a tenant. Whole-object dedup across tenants leaks only existence (the classic dedup side channel), and only if a client can see whether a write was new. **Status:** measured in E126; the cost of per-tenant stores is in its log entry.

**S11. A reverse-link view [inf].** `used_by` scans every stored object. **Status:** confirmed by E125: 113,210 reads, 260 MB and 340 ms on the cases store against 8 reads, 12.8 KB and about 1 ms from a view keyed by target; the full build took 40 s for 329k edges, so it must be maintained per write, as the db views are.

## Unknowns

- **Bitcask paper:** sources.debian.org returned an anti-bot challenge page, so field layouts come only from a summary.
- **Pages that failed:**
  - The current docs.lakefs.io internals page and the Apple forum thread on APFS `rename()` durability both returned client errors.
  - Only the goals section of the Iceberg spec was reached. Delta Lake was not tried.
- **Rust std today:** read the PR and merge diff only; current `sync_all` behaviour unverified.
- **F_BARRIERFSYNC:** no primary Apple source read. The "1000x" figure is second-hand.
- **APFS:** no source for small-file overhead or large-directory scaling.
- **LMDB:** I did not read its own caveats section (no-shrink, network filesystems).
- **Not read:** redb's value sizes, dbt's "state comparison caveats" page, R2's LIST page size, and the last ~15% of O'Neil.
- R2 cost figures are [inf]; every hypothesis is unverified until E122 runs it.

## What the bake-off should measure because of this note

- Ingest objects/s on APFS for three settings: per-object F_FULLFSYNC (today), per-ref F_FULLFSYNC, and a per-batch barrier (S1).
- Power-loss correctness: every object reachable from every ref is present and hash-valid, and torn refs are detected (S1, S5).
- Device-written and on-disk bytes ÷ logical bytes, with RocksDB and fjall run with separation on and off (S2, S4).
- `ids_with_prefix` and random `read` latency, cold and warm, at 1M and 10M objects (S2, S3).
- Reclaim time and space after deleting 30% of objects (S2; sources 6, 10).
- Open/recovery time and RSS at 10M objects (S3).
- Simulated R2 cost per GB ingested and per 1k reads, loose vs packed (S6).
- Byte equality of incremental vs full view rebuilds per backend, and WebDAV p99 latency for materialized vs render-on-read views (S7, S8).
